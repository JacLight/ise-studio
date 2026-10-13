// Routing constraints in the Electronics workspace: net classes with impedance
// targets (the width and gap per layer worked out from the stackup by the
// engine), differential pairs with their lengths, skew and coupling, length-
// matching groups, every net's length, and the length-tuning panel the board
// opens on a track. Edits go back as pcb.netclass.set, pcb.pair.set,
// pcb.length.group and pcb.tune.

import { useEffect, useState } from "react";
import "./PcbSignal.css";

type P = [number, number];
export type Line = { model: "microstrip"; h: number; er: number; t: number } | { model: "stripline"; h1: number; er1: number; h2: number; er2: number; t: number };
export type LayerTarget = { layer: string; line: Line; width: number; z0: number; gap?: number; zdiff?: number; delay: number; notes?: string[] };
export type ClassTargets = { name: string; impedance?: number; diff_impedance?: number; tolerance: number; layers: LayerTarget[] };
export type PairV = { name: string; p: string; n: string; detected: boolean; class: string; length_p: number; length_n: number; skew: number; coupled: number; uncoupled: number; gap?: number; gap_min?: number; gap_max?: number; zdiff?: number; max_skew?: number; max_uncoupled?: number; routed: boolean };
export type NetLength = { net: string; length: number; tracks: number; vias: number; via_length: number; pad_to_die?: number; delay: number; routed: boolean };
export type GroupV = { name: string; members: { net: string; length: number; routed: boolean }[]; target: number; tolerance?: number; min?: number; max?: number; max_skew?: number; skew: number };
export type SignalV = { classes: ClassTargets[]; pairs: PairV[]; lengths: NetLength[]; groups: GroupV[]; pad_to_die?: Record<string, number>; detect_pairs: boolean };
export type NetClassV = { name: string; nets: string[]; patterns?: string[]; track_width: number; clearance: number; via_drill: number; via_diameter: number; impedance?: number; diff_impedance?: number; diff_gap?: number; impedance_tolerance?: number; max_uncoupled?: number; max_skew?: number; pair_via_pitch?: number };
export type TuneEdit = { id: string; points: P[]; bulges?: number[] };
export type TunePlan = { ok: boolean; edits: TuneEdit[]; nets: { net: string; before: number; after: number; target: number }[]; skew?: number; notes: string[] };

type Commit = (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>;
const F = (v: number | undefined | null, d = 3) => (v == null ? "—" : Math.abs(v) < 1e-9 ? "0" : v.toFixed(d).replace(/\.?0+$/, ""));
const LAYER = (l: string) => (l === "top" ? "Top" : l === "bottom" ? "Bottom" : l.replace(/^in/, "In"));

/** One net class: its rules and electrical targets, edited in place; the widths per layer below. */
function ClassCard(props: { boardId: string; c: NetClassV; t?: ClassTargets; commit: Commit; canEdit: boolean; members: number }) {
  const { c, t } = props;
  const blank = (v?: number) => (v == null ? "" : String(v));
  const [f, setF] = useState(() => ({ patterns: (c.patterns ?? []).join(" "), track_width: String(c.track_width), clearance: String(c.clearance), impedance: blank(c.impedance), diff_impedance: blank(c.diff_impedance), diff_gap: blank(c.diff_gap), impedance_tolerance: blank(c.impedance_tolerance), max_uncoupled: blank(c.max_uncoupled), max_skew: blank(c.max_skew) }));
  const key = JSON.stringify(c);
  useEffect(() => setF({ patterns: (c.patterns ?? []).join(" "), track_width: String(c.track_width), clearance: String(c.clearance), impedance: blank(c.impedance), diff_impedance: blank(c.diff_impedance), diff_gap: blank(c.diff_gap), impedance_tolerance: blank(c.impedance_tolerance), max_uncoupled: blank(c.max_uncoupled), max_skew: blank(c.max_skew) }), [key]); // eslint-disable-line react-hooks/exhaustive-deps
  const apply = () => {
    const num = (s: string) => (s.trim() === "" ? 0 : +s);
    void props.commit(
      "pcb.netclass.set",
      {
        board: props.boardId,
        name: c.name,
        patterns: f.patterns.split(/[\s,]+/).filter(Boolean),
        track_width: +f.track_width,
        clearance: +f.clearance,
        impedance: num(f.impedance),
        diff_impedance: num(f.diff_impedance),
        diff_gap: num(f.diff_gap),
        impedance_tolerance: num(f.impedance_tolerance),
        max_uncoupled: num(f.max_uncoupled),
        max_skew: num(f.max_skew),
      },
      `Net class ${c.name} set`,
    );
  };
  const field = (k: keyof typeof f, label: string, unit: string, hint?: string) => (
    <label className="ps-field" title={hint}>
      <span>{label}</span>
      <input value={f[k]} disabled={!props.canEdit} placeholder="—" onChange={(e) => setF({ ...f, [k]: e.target.value })} onKeyDown={(e) => e.key === "Enter" && apply()} aria-label={`${c.name} ${label}`} />
      <i>{unit}</i>
    </label>
  );
  const dirty = JSON.stringify(f) !== JSON.stringify({ patterns: (c.patterns ?? []).join(" "), track_width: String(c.track_width), clearance: String(c.clearance), impedance: blank(c.impedance), diff_impedance: blank(c.diff_impedance), diff_gap: blank(c.diff_gap), impedance_tolerance: blank(c.impedance_tolerance), max_uncoupled: blank(c.max_uncoupled), max_skew: blank(c.max_skew) });
  return (
    <div className="ps-class">
      <div className="ps-class-head">
        <b>{c.name}</b>
        <small>{props.members} net{props.members === 1 ? "" : "s"}</small>
        {props.canEdit && c.name.toLowerCase() !== "default" && (
          <button type="button" className="ps-x" onClick={() => void props.commit("pcb.netclass.set", { board: props.boardId, name: c.name, delete: true }, `Net class ${c.name} removed`)} aria-label={`Remove ${c.name}`}>
            ×
          </button>
        )}
      </div>
      <div className="ps-grid">
        {field("patterns", "Nets", "", "Net names or patterns: USB_*, D?")}
        {field("track_width", "Width", "mm", "Width when no impedance target is set")}
        {field("clearance", "Clearance", "mm")}
        {field("impedance", "Z single", "Ω", "Single-ended impedance target")}
        {field("diff_impedance", "Z diff", "Ω", "Differential impedance target for the class's pairs")}
        {field("diff_gap", "Pair gap", "mm", "Fixed gap (the width is solved for it); blank: solved")}
        {field("impedance_tolerance", "Tolerance", "%", "Default 10 %")}
        {field("max_uncoupled", "Uncoupled", "mm", "Longest stretch a pair may run apart")}
        {field("max_skew", "Skew", "mm", "Largest length difference within a pair")}
      </div>
      {dirty && (
        <div className="pcb-row-actions">
          <button type="button" className="small primary" disabled={!props.canEdit} title={`Save these settings for net class ${c.name}`} onClick={apply}>
            Apply
          </button>
        </div>
      )}
      {t && (
        <table className="ps-table">
          <thead>
            <tr>
              <th>Layer</th>
              <th>Model</th>
              <th>Width</th>
              {t.diff_impedance != null && <th>Gap</th>}
              <th>Z0</th>
              {t.diff_impedance != null && <th>Zdiff</th>}
              <th>ps/mm</th>
            </tr>
          </thead>
          <tbody>
            {t.layers.map((l) => (
              <tr key={l.layer} className={l.notes?.length ? "warn" : ""} title={l.notes?.join("\n")}>
                <td>{LAYER(l.layer)}</td>
                <td>{l.line.model === "microstrip" ? `μstrip h${F(l.line.h)}` : `stripline ${F(l.line.h1)}/${F(l.line.h2)}`}</td>
                <td>
                  <b>{F(l.width)}</b>
                </td>
                {t.diff_impedance != null && <td>{F(l.gap)}</td>}
                <td>{F(l.z0, 1)}</td>
                {t.diff_impedance != null && <td>{F(l.zdiff, 1)}</td>}
                <td>{F(l.delay, 2)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** Net classes, pairs, length groups and net lengths: the board's routing constraints. */
export function SignalPanel(props: { boardId: string; classes: NetClassV[]; nets: { name: string; class: string }[]; s: SignalV; violations: { code: string; message: string; severity: string }[]; commit: Commit; canEdit: boolean; onNet: (n: string) => void }) {
  const { s, commit, canEdit, boardId } = props;
  const [newClass, setNewClass] = useState("");
  const [group, setGroup] = useState({ name: "", nets: "", target: "", tolerance: "", max_skew: "" });
  const [pair, setPair] = useState({ name: "", p: "", n: "" });
  const hits = props.violations.filter((v) => ["impedance", "diff_pair_gap", "diff_pair_uncoupled", "length", "skew", "via_count"].includes(v.code));
  const len = (n: string) => s.lengths.find((l) => l.net === n);
  return (
    <div className="ps">
      <h4 className="pcb-h">Net classes</h4>
      <p className="pcb-note">Impedance targets set the width (and a pair's gap) on every layer from the stackup: microstrip outside, stripline inside, the next layers as planes.</p>
      {props.classes.map((c) => (
        <ClassCard key={c.name} boardId={boardId} c={c} t={s.classes.find((x) => x.name === c.name)} commit={commit} canEdit={canEdit} members={props.nets.filter((n) => n.class === c.name).length} />
      ))}
      {canEdit && (
        <div className="pcb-row-actions">
          <input className="ps-input" value={newClass} onChange={(e) => setNewClass(e.target.value)} placeholder="New class (USB, DDR…)" aria-label="New net class" />
          <button
            type="button"
            className="small"
            disabled={!newClass.trim()}
            title="Add a net class with the name typed"
            onClick={() => {
              void commit("pcb.netclass.set", { board: boardId, name: newClass.trim() }, `Net class ${newClass.trim()} added`);
              setNewClass("");
            }}
          >
            Add class
          </button>
        </div>
      )}

      <h4 className="pcb-h">Differential pairs</h4>
      {s.pairs.length === 0 && <p className="pcb-note">None: nets named D+/D−, _P/_N or DP/DM pair up by themselves, or name a pair below.</p>}
      {s.pairs.length > 0 && (
        <table className="ps-table">
          <thead>
            <tr>
              <th>Pair</th>
              <th>Length</th>
              <th>Skew</th>
              <th>Uncoupled</th>
              <th>Gap</th>
              <th>Zdiff</th>
            </tr>
          </thead>
          <tbody>
            {s.pairs.map((p) => (
              <tr key={p.name} onClick={() => props.onNet(p.p)} className="click">
                <td title={`${p.p} / ${p.n}${p.detected ? " (found from the names)" : ""} · class ${p.class}`}>
                  <b>{p.name}</b>
                </td>
                <td>
                  {F(p.length_p, 2)} / {F(p.length_n, 2)}
                </td>
                <td className={p.max_skew != null && p.skew > p.max_skew + 1e-6 ? "bad" : ""}>
                  {F(p.skew)}
                  {p.max_skew != null && <small> ≤{F(p.max_skew)}</small>}
                </td>
                <td className={p.max_uncoupled != null && p.uncoupled > p.max_uncoupled + 1e-6 ? "bad" : ""}>
                  {F(p.uncoupled, 2)}
                  {p.max_uncoupled != null && <small> ≤{F(p.max_uncoupled)}</small>}
                </td>
                <td>{p.gap_min != null ? (Math.abs((p.gap_max ?? 0) - p.gap_min) < 1e-4 ? F(p.gap_min) : `${F(p.gap_min)}–${F(p.gap_max)}`) : F(p.gap)}</td>
                <td>{p.zdiff != null ? `${F(p.zdiff, 1)} Ω` : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {canEdit && (
        <div className="pcb-row-actions">
          <input className="ps-input" value={pair.name} onChange={(e) => setPair({ ...pair, name: e.target.value })} placeholder="Pair" aria-label="Pair name" />
          <input className="ps-input" value={pair.p} onChange={(e) => setPair({ ...pair, p: e.target.value })} placeholder="+ net" aria-label="Positive net" />
          <input className="ps-input" value={pair.n} onChange={(e) => setPair({ ...pair, n: e.target.value })} placeholder="− net" aria-label="Negative net" />
          <button
            type="button"
            className="small"
            disabled={!pair.name || !pair.p || !pair.n}
            title="Make the + and − nets a differential pair under this name"
            onClick={() => {
              void commit("pcb.pair.set", { board: boardId, ...pair }, `Pair ${pair.name} set`);
              setPair({ name: "", p: "", n: "" });
            }}
          >
            Pair
          </button>
        </div>
      )}

      <h4 className="pcb-h">Length matching</h4>
      {s.groups.map((g) => (
        <div key={g.name} className="ps-group">
          <div className="ps-class-head">
            <b>{g.name}</b>
            <small>
              target {F(g.target)} mm{g.tolerance != null ? ` ± ${F(g.tolerance)}` : ""}
              {g.max_skew != null ? ` · spread ≤ ${F(g.max_skew)}` : ""} · spread {F(g.skew)}
            </small>
            {canEdit && (
              <button type="button" className="ps-x" onClick={() => void commit("pcb.length.group", { board: boardId, name: g.name, delete: true }, `Group ${g.name} removed`)} aria-label={`Remove ${g.name}`}>
                ×
              </button>
            )}
          </div>
          <div className="ps-bars">
            {g.members.map((m) => {
              const off = g.target - m.length;
              const ok = g.tolerance == null || Math.abs(off) <= g.tolerance + 1e-6;
              return (
                <button key={m.net} type="button" className={`ps-bar ${ok ? "" : "bad"}`} onClick={() => props.onNet(m.net)} title={`${m.net}: ${F(m.length)} mm (${off > 0 ? `${F(off)} short` : "at target"})`}>
                  <span>{m.net}</span>
                  <i style={{ width: `${g.target > 0 ? Math.min(100, (100 * m.length) / g.target) : 0}%` }} />
                  <em>{F(m.length, 2)}</em>
                </button>
              );
            })}
          </div>
          {canEdit && (
            <div className="pcb-row-actions">
              <button type="button" className="small" onClick={() => void commit("pcb.tune", { board: boardId, group: g.name }, `${g.name} tuned`)} title="Meander every member to the target (or the longest)">
                Tune all
              </button>
            </div>
          )}
        </div>
      ))}
      {canEdit && (
        <div className="ps-grid">
          <label className="ps-field">
            <span>Group</span>
            <input value={group.name} onChange={(e) => setGroup({ ...group, name: e.target.value })} placeholder="BUS" />
          </label>
          <label className="ps-field wide">
            <span>Nets</span>
            <input value={group.nets} onChange={(e) => setGroup({ ...group, nets: e.target.value })} placeholder="D0 D1 D2 D3, D?, or a pair" />
          </label>
          <label className="ps-field">
            <span>Target</span>
            <input value={group.target} onChange={(e) => setGroup({ ...group, target: e.target.value })} placeholder="longest" />
            <i>mm</i>
          </label>
          <label className="ps-field">
            <span>±</span>
            <input value={group.tolerance} onChange={(e) => setGroup({ ...group, tolerance: e.target.value })} placeholder="0.5" />
            <i>mm</i>
          </label>
          <label className="ps-field">
            <span>Spread</span>
            <input value={group.max_skew} onChange={(e) => setGroup({ ...group, max_skew: e.target.value })} placeholder="—" />
            <i>mm</i>
          </label>
          <button
            type="button"
            className="small"
            disabled={!group.name || !group.nets}
            title="Group these nets to be length-matched to the target"
            onClick={() => {
              const n = (s: string) => (s.trim() ? +s : undefined);
              void commit("pcb.length.group", { board: boardId, name: group.name, nets: group.nets.split(/[\s,]+/).filter(Boolean), target: n(group.target), tolerance: n(group.tolerance), max_skew: n(group.max_skew) }, `Group ${group.name} set`);
              setGroup({ name: "", nets: "", target: "", tolerance: "", max_skew: "" });
            }}
          >
            Add group
          </button>
        </div>
      )}

      {hits.length > 0 && (
        <>
          <h4 className="pcb-h">Constraint violations</h4>
          <ul className="ps-hits">
            {hits.map((v, i) => (
              <li key={i} className={v.severity}>
                <b>{v.code.replace(/_/g, " ")}</b> {v.message}
              </li>
            ))}
          </ul>
        </>
      )}

      <h4 className="pcb-h">Net lengths</h4>
      <table className="ps-table">
        <thead>
          <tr>
            <th>Net</th>
            <th>Length</th>
            <th>Vias</th>
            <th>Delay</th>
          </tr>
        </thead>
        <tbody>
          {s.lengths.map((l) => (
            <tr key={l.net} className="click" onClick={() => props.onNet(l.net)}>
              <td>{l.net}</td>
              <td title={`track ${F(l.tracks)} + vias ${F(l.via_length)}${l.pad_to_die ? ` + die ${F(l.pad_to_die)}` : ""}`}>
                <b>{F(len(l.net)?.length, 3)}</b>
              </td>
              <td>{l.vias ? `${l.vias} · ${F(l.via_length, 2)}` : "—"}</td>
              <td>{F(l.delay, 1)} ps</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** The length-tuning tool on the board: a target, the meander's shape, a live preview and the result. */
export function TunePanel(props: {
  boardId: string;
  net: string;
  pair?: PairV;
  groupTarget?: number;
  plan: TunePlan | null;
  onAsk: (spec: Record<string, unknown>) => void;
  onApply: (spec: Record<string, unknown>) => void;
  onClose: () => void;
  trackId: string;
}) {
  const partner = props.pair ? (props.pair.p === props.net ? props.pair.length_n : props.pair.length_p) : undefined;
  const [target, setTarget] = useState(() => F(props.groupTarget ?? partner ?? 0, 3));
  const [mode, setMode] = useState<"net" | "pair" | "skew">(props.pair ? (props.groupTarget ? "pair" : "skew") : "net");
  const [amp, setAmp] = useState("1");
  const [spacing, setSpacing] = useState("");
  const [style, setStyle] = useState<"trombone" | "sawtooth">("trombone");
  const [corner, setCorner] = useState<"round" | "chamfer" | "square">("round");
  const [onlyHere, setOnlyHere] = useState(false);
  const spec = () => ({
    board: props.boardId,
    ...(mode === "net" ? { net: props.net } : { pair: props.pair!.name }),
    ...(mode === "skew" ? { mode: "skew" } : { target: +target }),
    amplitude: +amp,
    ...(spacing ? { spacing: +spacing } : {}),
    style,
    corner,
    ...(onlyHere && mode === "net" ? { track: props.trackId } : {}),
  });
  useEffect(() => {
    if (mode !== "skew" && !(+target > 0)) return;
    props.onAsk(spec());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, mode, amp, spacing, style, corner, onlyHere]);
  const p = props.plan;
  return (
    <div className="ps-tune" role="dialog" aria-label="Length tuning">
      <div className="ps-tune-head">
        <b>Tune {mode === "net" ? props.net : props.pair?.name}</b>
        <button type="button" className="ps-x" onClick={props.onClose} aria-label="Close tuning">
          ×
        </button>
      </div>
      {props.pair && (
        <div className="ps-seg">
          {(["pair", "skew", "net"] as const).map((m) => (
            <button key={m} type="button" className={mode === m ? "on" : ""} title={m === "pair" ? "Meander both nets of the pair to the target length" : m === "skew" ? "Meander the shorter net only, to match the longer" : "Meander only this net to the target"} onClick={() => setMode(m)}>
              {m === "pair" ? "Pair length" : m === "skew" ? "Skew only" : "This track"}
            </button>
          ))}
        </div>
      )}
      <div className="ps-tune-grid">
        {mode !== "skew" && (
          <label>
            <span>Target</span>
            <input value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Target length mm" />
            <i>mm</i>
          </label>
        )}
        <label>
          <span>Amplitude</span>
          <input value={amp} onChange={(e) => setAmp(e.target.value)} aria-label="Amplitude mm" />
          <i>mm</i>
        </label>
        <label>
          <span>Spacing</span>
          <input value={spacing} onChange={(e) => setSpacing(e.target.value)} placeholder="auto" aria-label="Spacing mm" />
          <i>mm</i>
        </label>
        <label>
          <span>Style</span>
          <select value={style} title="Meander shape (single tracks only)" onChange={(e) => setStyle(e.target.value as typeof style)} disabled={mode !== "net"}>
            <option value="trombone">Trombone</option>
            <option value="sawtooth">Sawtooth</option>
          </select>
        </label>
        <label>
          <span>Corners</span>
          <select value={corner} title="Corner shape of the meanders" onChange={(e) => setCorner(e.target.value as typeof corner)}>
            <option value="round">Round</option>
            <option value="chamfer">45°</option>
            <option value="square">Square</option>
          </select>
        </label>
        {mode === "net" && (
          <label className="ps-check">
            <input type="checkbox" checked={onlyHere} title="Meander only the picked track segment, not the whole net" onChange={(e) => setOnlyHere(e.target.checked)} /> Only this track
          </label>
        )}
      </div>
      <div className="ps-readout">
        {p ? (
          <>
            {p.nets.map((n) => (
              <div key={n.net} className={Math.abs(n.after - n.target) <= 0.01 ? "ok" : "bad"}>
                <span>{n.net}</span>
                <b>
                  {F(n.before, 3)} → {F(n.after, 3)}
                </b>
                <small>/ {F(n.target, 3)} mm</small>
              </div>
            ))}
            {p.skew != null && <div className={p.skew <= 0.01 ? "ok" : "bad"}>skew {F(p.skew, 4)} mm</div>}
            {p.notes.map((n, i) => (
              <p key={i} className="pcb-note">
                {n}
              </p>
            ))}
          </>
        ) : (
          <p className="pcb-note">Working out meanders…</p>
        )}
      </div>
      <div className="pcb-row-actions">
        <button type="button" className="small primary" disabled={!p || !p.edits.length} title="Add the previewed meanders to the board" onClick={() => props.onApply(spec())}>
          Apply
        </button>
        <span className="pcb-note">{p ? `${p.edits.length} track${p.edits.length === 1 ? "" : "s"} meandered` : ""}</span>
      </div>
    </div>
  );
}
