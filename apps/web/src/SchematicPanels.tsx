// Side panels of the schematic view beyond parts and nets: properties of a
// selected label, note, bus or graphic; the sheet hierarchy; the field table;
// schematic setup (net classes, bus aliases, ERC matrix and severities, field
// templates, text variables, title block and drawing sheet, annotation);
// exports; and find & replace. Every change is a pcb.schematic.* command.

import { useMemo, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import type { Commit, Erc, GraphicV, LabelKind, SchematicV, Sel, SymbolV } from "./schematicTypes";

const COLORS = ["#d03030", "#e7861b", "#c9a400", "#1b7f3d", "#0d6b6b", "#2a5bd7", "#7a3aa8", "#3c4a63"];

function Field(props: { label: string; value: string; onSave: (v: string) => void; disabled?: boolean; placeholder?: string; multiline?: boolean }) {
  const common = {
    defaultValue: props.value,
    disabled: props.disabled,
    placeholder: props.placeholder,
    onBlur: (e: { target: { value: string } }) => {
      if (e.target.value !== props.value) props.onSave(e.target.value);
    },
  };
  return (
    <label className="sch-field">
      <span>{props.label}</span>
      {props.multiline ? (
        <textarea key={props.value} rows={3} {...common} />
      ) : (
        <input key={props.value} {...common} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      )}
    </label>
  );
}

function Swatches(props: { value: string | null | undefined; onPick: (c: string) => void; disabled?: boolean; allowNone?: boolean }) {
  return (
    <div className="schx-swatches">
      {props.allowNone && (
        <button type="button" className={`schx-swatch none ${!props.value ? "on" : ""}`} disabled={props.disabled} title="None" onClick={() => props.onPick("")} />
      )}
      {COLORS.map((c) => (
        <button key={c} type="button" className={`schx-swatch ${props.value === c ? "on" : ""}`} style={{ background: c }} disabled={props.disabled} title={c} onClick={() => props.onPick(c)} />
      ))}
      <input type="color" className="schx-color" value={props.value || "#3c4a63"} disabled={props.disabled} onChange={(e) => props.onPick(e.target.value)} title="Any colour" />
    </div>
  );
}

// ---- Properties of a selected non-symbol item ------------------------------------------------

export function ItemProps(props: { v: SchematicV; sel: NonNullable<Sel>; commit: Commit; canEdit: boolean; onBack: () => void; sheet: string }) {
  const { v, sel, commit, canEdit } = props;
  const upd = (p: Record<string, unknown>) => void commit("pcb.schematic.item.update", { target: sel.id, ...p });
  const head = (title: string, sub?: string) => (
    <>
      <button type="button" className="sch-back" onClick={props.onBack}>
        ← All parts
      </button>
      <div className="sch-props-head">
        <strong>{title}</strong>
        {sub && <span>{sub}</span>}
      </div>
    </>
  );
  if (sel.kind === "label") {
    const l = v.labels.find((x) => x.id === sel.id);
    if (!l) return null;
    const kinds: LabelKind[] = ["local", "global", "hierarchical", "power", "directive"];
    return (
      <div className="pcb-tab sch-props">
        {head(l.text, `${l.kind} label${l.net ? ` · net ${l.net}` : ""}`)}
        <Field label="Text" value={l.text} disabled={!canEdit} onSave={(t) => t.trim() && upd({ text: t })} />
        <label className="sch-field">
          <span>Kind</span>
          <select value={l.kind} disabled={!canEdit} onChange={(e) => upd({ kind: e.target.value })}>
            {kinds.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </label>
        {(l.kind === "global" || l.kind === "hierarchical") && (
          <label className="sch-field">
            <span>Shape</span>
            <select defaultValue="" disabled={!canEdit} onChange={(e) => upd({ shape: e.target.value })}>
              {["", "input", "output", "bidirectional", "tri_state", "passive"].map((k) => (
                <option key={k} value={k}>
                  {k || "default"}
                </option>
              ))}
            </select>
          </label>
        )}
        <div className="sch-props-row">
          <button type="button" className="secondary" disabled={!canEdit} onClick={() => upd({ rotation: (l.rotation + 90) % 360 })} title="Turn (R)">
            <Icon name="rotate" />
          </button>
        </div>
      </div>
    );
  }
  if (sel.kind === "text") {
    const t = v.texts.find((x) => x.id === sel.id);
    if (!t) return null;
    return (
      <div className="pcb-tab sch-props">
        {head("Note", t.text.split("\n")[0])}
        <Field label="Text" value={t.source} multiline disabled={!canEdit} onSave={(x) => x.trim() && upd({ text: x })} />
        <Field label="Size" value={String(t.size)} disabled={!canEdit} onSave={(x) => upd({ size: Number(x) })} />
        <div className="schx-seg">
          {[
            ["left", "Left"],
            ["center", "Centre"],
            ["right", "Right"],
          ].map(([k, label]) => (
            <button key={k} type="button" className={(t.justify || "left") === k ? "on" : ""} disabled={!canEdit} onClick={() => upd({ justify: k })}>
              {label}
            </button>
          ))}
          <button type="button" className={t.bold ? "on" : ""} disabled={!canEdit} onClick={() => upd({ bold: !t.bold })}>
            <b>B</b>
          </button>
          <button type="button" className={t.italic ? "on" : ""} disabled={!canEdit} onClick={() => upd({ italic: !t.italic })}>
            <i>I</i>
          </button>
          <button type="button" className={t.angle ? "on" : ""} disabled={!canEdit} onClick={() => upd({ angle: t.angle ? 0 : 90 })} title="Vertical">
            ⤒
          </button>
        </div>
        <Swatches value={t.color} allowNone disabled={!canEdit} onPick={(c) => upd({ color: c })} />
        <p className="small sch-hint">Text variables expand: {"${TITLE}"}, {"${REVISION}"}, {"${DATE}"}, {"${SHEETNAME}"}, {"${#}"} and your own (Setup).</p>
      </div>
    );
  }
  if (sel.kind === "bus") {
    const b = v.buses.find((x) => x.id === sel.id && x.sheet === props.sheet) ?? v.buses.find((x) => x.id === sel.id);
    if (!b) return null;
    const out = new Set(v.labels.filter((l) => l.sheet === b.sheet && l.kind === "local").map((l) => l.text));
    return (
      <div className="pcb-tab sch-props">
        {head(b.name ?? "Unnamed bus", b.name ? `${b.members.length} members` : "Put a bus label on it (N): D[0..7], {SDA SCL}")}
        <div className="schx-members">
          {b.members.map((m) => (
            <div key={m.member} className="schx-member">
              <span className="m">{m.member}</span>
              <span className={`n ${m.net ? "" : "none"}`}>{m.net ?? "unused"}</span>
              {!out.has(m.member) && (
                <button type="button" className="secondary" disabled={!canEdit} onClick={() => void commit("pcb.schematic.bus.unfold", { bus: b.id, members: [m.member] }, `${m.member} broken out`)}>
                  Break out
                </button>
              )}
            </div>
          ))}
        </div>
        {b.name && (
          <button type="button" className="primary" disabled={!canEdit} onClick={() => void commit("pcb.schematic.bus.unfold", { bus: b.id }, "Members broken out")}>
            Break out every member
          </button>
        )}
      </div>
    );
  }
  if (sel.kind === "graphic") {
    const g = v.graphics.find((x) => x.id === sel.id);
    if (!g) return null;
    return <GraphicProps g={g} commit={commit} canEdit={canEdit} head={head} />;
  }
  return (
    <div className="pcb-tab sch-props">
      {head(sel.kind === "wire" ? "Wire" : sel.kind === "entry" ? "Bus entry" : sel.kind === "junction" ? "Junction" : "No-connect flag", sel.kind === "wire" ? (v.wires.find((w) => w.id === sel.id)?.net ?? "no net") : undefined)}
      {sel.kind === "entry" && (
        <div className="sch-props-row">
          <button type="button" className="secondary" disabled={!canEdit} onClick={() => void commit("pcb.schematic.move", { targets: [sel.id], mirror: "x" })}>
            Flip
          </button>
        </div>
      )}
      <p className="small sch-hint">Drag to move · Del to delete · R turn · X / Y mirror.</p>
    </div>
  );
}

function GraphicProps(props: { g: GraphicV; commit: Commit; canEdit: boolean; head: (t: string, s?: string) => React.ReactNode }) {
  const { g, commit, canEdit } = props;
  const set = (s: Record<string, unknown>) => void commit("pcb.schematic.item.update", { target: g.id, set: s });
  const styled = g.draw.find((d) => d.t === "styled");
  const color = styled && styled.t === "styled" ? styled.color : null;
  const entity = g.props;
  const title = { shape: "Shape", text_box: "Text box", table: "Table", image: "Picture" }[g.kind];
  return (
    <div className="pcb-tab sch-props">
      {props.head(title, g.kind === "shape" ? String(entity?.shape ?? "") : undefined)}
      {g.kind === "text_box" && entity && <Field label="Text" value={String(entity.text ?? "")} multiline disabled={!canEdit} onSave={(x) => set({ text: x })} />}
      {g.kind === "table" && entity && <TableCells cells={(entity.cells as string[][]) ?? []} onSave={(cells) => set({ cells })} disabled={!canEdit} />}
      {g.kind === "image" && entity && <Field label="Width mm" value={String(entity.width ?? 50)} disabled={!canEdit} onSave={(x) => set({ width: Number(x) })} />}
      {g.kind !== "image" && (
        <>
          <h4 className="sch-sub">Line</h4>
          <Swatches value={color} disabled={!canEdit} onPick={(c) => set({ color: c || null })} />
          {g.kind === "shape" && (
            <div className="schx-seg">
              {[
                ["solid", "—"],
                ["dash", "- -"],
                ["dot", "···"],
                ["dash_dot", "-·-"],
              ].map(([k, l]) => (
                <button key={k} type="button" className={(entity?.dash || "solid") === k ? "on" : ""} disabled={!canEdit} onClick={() => set({ dash: k })}>
                  {l}
                </button>
              ))}
            </div>
          )}
          <Field label="Width mm" value={String(entity?.width ?? 0.254)} disabled={!canEdit} onSave={(x) => set({ width: Number(x) })} />
          {(g.kind === "shape" || g.kind === "text_box") && (
            <>
              <h4 className="sch-sub">Fill</h4>
              <Swatches value={(entity?.fill as string) ?? null} allowNone disabled={!canEdit} onPick={(c) => set({ fill: c || null })} />
            </>
          )}
        </>
      )}
    </div>
  );
}

function TableCells(props: { cells: string[][]; onSave: (c: string[][]) => void; disabled?: boolean }) {
  const [cells, setCells] = useState(props.cells);
  const cols = Math.max(1, ...cells.map((r) => r.length));
  const edit = (r: number, c: number, val: string) => setCells(cells.map((row, i) => (i === r ? Array.from({ length: cols }, (_, j) => (j === c ? val : (row[j] ?? ""))) : row)));
  return (
    <div className="schx-table-edit">
      <table>
        <tbody>
          {cells.map((row, r) => (
            <tr key={r}>
              {Array.from({ length: cols }, (_, c) => (
                <td key={c}>
                  <input value={row[c] ?? ""} disabled={props.disabled} onChange={(e) => edit(r, c, e.target.value)} onBlur={() => props.onSave(cells)} />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="sch-props-row">
        <button type="button" className="secondary" disabled={props.disabled} onClick={() => props.onSave([...cells, Array(cols).fill("")])}>
          + Row
        </button>
        <button type="button" className="secondary" disabled={props.disabled} onClick={() => props.onSave(cells.map((r) => [...r, ""]))}>
          + Column
        </button>
        <button type="button" className="secondary" disabled={props.disabled || cells.length < 2} onClick={() => props.onSave(cells.slice(0, -1))}>
          − Row
        </button>
      </div>
    </div>
  );
}

// ---- Hierarchy ------------------------------------------------------------------------

export function SheetsPanel(props: { v: SchematicV; sheet: string; onSheet: (s: string) => void; commit: Commit; canEdit: boolean }) {
  const { v, commit, canEdit } = props;
  const depth = (id: string, n = 0): number => {
    const s = v.sheets.find((x) => x.id === id);
    return s?.parent && n < 16 ? depth(s.parent, n + 1) + 1 : 0;
  };
  // Tree order: each sheet followed by its children.
  const order: string[] = [];
  const walk = (parent: string | null) => {
    for (const s of v.sheets.filter((x) => x.parent === parent)) {
      order.push(s.id);
      walk(s.id);
    }
  };
  walk(null);
  for (const s of v.sheets) if (!order.includes(s.id)) order.push(s.id);
  const cur = v.sheets.find((s) => s.id === props.sheet);
  const used = (id: string) => v.symbols.some((s) => s.sheet === id && !s.instance) || v.wires.some((w) => w.sheet === id);
  const addSheet = async (instanceOf?: string) => {
    if (!cur) return;
    const n = v.sheets.filter((s) => s.parent === cur.id).length;
    const at: [number, number] = [38.1 + (n % 4) * 50.8, 38.1 + Math.floor(n / 4) * 38.1];
    const src = instanceOf ? v.sheets.find((s) => s.id === instanceOf) : null;
    const out = (await commit("pcb.schematic.sheet", { schematic: v.id, parent: cur.id, name: src ? `${src.name} ${String.fromCharCode(66 + v.sheets.filter((s) => s.instance_of === instanceOf).length)}` : `Sheet ${v.sheets.length + 1}`, at, size: [30.48, 20.32], ...(instanceOf ? { instance_of: instanceOf, sync_pins: true } : {}) }, instanceOf ? "Another instance added" : "Sheet added")) as { output?: { sheet: string } } | null;
    if (out?.output?.sheet && !instanceOf) props.onSheet(out.output.sheet);
  };
  return (
    <div className="pcb-tab">
      <div className="schx-tree">
        {order.map((id) => {
          const s = v.sheets.find((x) => x.id === id)!;
          const syms = v.symbols.filter((x) => x.sheet === id).length;
          return (
            <button key={id} type="button" className={`pcb-row ${id === props.sheet ? "active" : ""}`} style={{ paddingLeft: 10 + depth(id) * 16 }} onClick={() => props.onSheet(id)}>
              <span className="ref">{s.page}</span>
              <span className="val">
                {s.name}
                <small>
                  {syms} symbol{syms === 1 ? "" : "s"}
                  {s.instance_of ? ` · instance of ${v.sheets.find((x) => x.id === s.instance_of)?.name}` : ""}
                </small>
              </span>
            </button>
          );
        })}
      </div>
      {cur && (
        <div className="schx-card">
          <h4 className="sch-sub">This sheet</h4>
          <Field label="Name" value={cur.name} disabled={!canEdit} onSave={(name) => name.trim() && void commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, name })} />
          <Field label="Page" value={cur.page} disabled={!canEdit} onSave={(page) => void commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, page })} />
          <label className="sch-field">
            <span>Paper</span>
            <select value={cur.paper} disabled={!canEdit} onChange={(e) => void commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, paper: e.target.value })}>
              {["A4", "A3", "A2", "A1", "Letter", "Tabloid"].map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label className="sch-check">
            <input type="checkbox" checked={cur.portrait} disabled={!canEdit} onChange={(e) => void commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, portrait: e.target.checked })} /> Portrait
          </label>
          <div className="sch-props-row wrap">
            <button type="button" className="secondary" disabled={!canEdit || !!cur.instance_of} onClick={() => void addSheet()}>
              + Sub-sheet
            </button>
            {cur.parent && (
              <button type="button" className="secondary" disabled={!canEdit} title="Make the box's pins match the hierarchical labels inside" onClick={() => void commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, sync_pins: true }, "Sheet pins match the labels")}>
                Sync pins
              </button>
            )}
            {cur.parent && !cur.instance_of && (
              <button
                type="button"
                className="secondary"
                disabled={!canEdit}
                title="Use this sheet's circuit again (own references and nets)"
                onClick={async () => {
                  const parent = cur.parent!;
                  const n = v.sheets.filter((s) => s.parent === parent).length;
                  await commit("pcb.schematic.sheet", { schematic: v.id, parent, name: `${cur.name} ${String.fromCharCode(66 + v.sheets.filter((s) => s.instance_of === cur.id).length)}`, instance_of: cur.id, at: [38.1 + (n % 4) * 50.8, 38.1 + Math.floor(n / 4) * 38.1], size: [30.48, 20.32], sync_pins: true }, "Another instance added");
                }}
              >
                + Instance
              </button>
            )}
            {v.sheets.length > 1 && (
              <button
                type="button"
                className="secondary danger"
                disabled={!canEdit || used(cur.id) || v.sheets.some((s) => s.parent === cur.id)}
                onClick={async () => {
                  const back = cur.parent ?? v.sheets.find((s) => s.id !== cur.id)!.id;
                  await commit("pcb.schematic.sheet", { schematic: v.id, id: cur.id, delete: true }, "Sheet deleted");
                  props.onSheet(back);
                }}
              >
                Delete
              </button>
            )}
          </div>
          {cur.parent && (
            <button type="button" className="sch-back" onClick={() => props.onSheet(cur.parent!)}>
              ↑ Up to {v.sheets.find((s) => s.id === cur.parent)?.name}
            </button>
          )}
        </div>
      )}
      <p className="small sch-hint">Double-click a sheet box to enter it; Backspace goes back up. An instance shows the same circuit with its own references (annotate numbers them).</p>
    </div>
  );
}

// ---- Field table ----------------------------------------------------------------------

const STD = ["Reference", "Value", "Footprint", "MPN", "Manufacturer", "LCSC", "Description", "DNP"];

function stdField(s: SymbolV, f: string): string {
  switch (f) {
    case "Reference":
      return s.reference;
    case "Value":
      return s.value;
    case "Footprint":
      return s.footprint;
    case "MPN":
      return s.mpn ?? "";
    case "Manufacturer":
      return s.manufacturer ?? "";
    case "LCSC":
      return s.lcsc ?? "";
    case "Description":
      return s.description;
    case "DNP":
      return s.dnp ? "DNP" : "";
    default:
      return s.fields?.[f] ?? "";
  }
}

export function FieldsPanel(props: { v: SchematicV; commit: Commit; canEdit: boolean; onSelect: (s: SymbolV) => void }) {
  const { v, commit, canEdit } = props;
  const [group, setGroup] = useState(true);
  const [extra, setExtra] = useState<string[]>([]);
  const [q, setQ] = useState("");
  const custom = useMemo(() => {
    const set = new Set<string>(extra);
    for (const t of v.setup.field_templates) set.add(t.name);
    for (const s of v.symbols) for (const k of Object.keys(s.fields ?? {})) set.add(k);
    return [...set].filter((k) => !STD.includes(k));
  }, [v, extra]);
  const cols = [...STD, ...custom];
  // One row per part (units merged), or per group of identical parts.
  const parts = useMemo(() => {
    const by = new Map<string, SymbolV>();
    for (const s of v.symbols) if (!s.virtual_part && !s.reference.startsWith("#")) by.set(`${s.reference}@${s.instance ?? ""}`, by.get(`${s.reference}@${s.instance ?? ""}`) ?? s);
    return [...by.values()].sort((a, b) => a.reference.localeCompare(b.reference, undefined, { numeric: true }));
  }, [v]);
  const rows = useMemo(() => {
    const filtered = parts.filter((s) => !q || cols.some((c) => stdField(s, c).toLowerCase().includes(q.toLowerCase())));
    if (!group) return filtered.map((s) => ({ key: s.id + s.sheet, members: [s] }));
    const g = new Map<string, SymbolV[]>();
    for (const s of filtered) {
      const k = [s.value, s.footprint, s.mpn ?? "", s.dnp ? "dnp" : ""].join("\u0001");
      g.set(k, [...(g.get(k) ?? []), s]);
    }
    return [...g.entries()].map(([key, members]) => ({ key, members }));
  }, [parts, group, q, cols]);
  const save = (members: SymbolV[], field: string, value: string) => {
    if (field === "Reference" && members.length === 1) {
      const s = members[0];
      void commit("pcb.schematic.fields", { schematic: v.id, set: [{ target: s.id, field, value, ...(s.instance ? { instance: s.sheet } : {}) }] });
      return;
    }
    void commit("pcb.schematic.fields", { schematic: v.id, set: members.map((s) => ({ reference: s.reference, field, value })) }, `${field} set on ${members.length} part${members.length === 1 ? "" : "s"}`);
  };
  const csv = () => {
    const cell = (x: string) => (/[",\n]/.test(x) ? `"${x.replace(/"/g, '""')}"` : x);
    const head = ["Qty", ...cols];
    const lines = [head.join(","), ...rows.map((r) => [String(r.members.length), ...cols.map((c) => (c === "Reference" ? r.members.map((m) => m.reference).join(" ") : stdField(r.members[0], c)))].map(cell).join(","))];
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([lines.join("\n") + "\n"], { type: "text/csv" }));
    a.download = `${v.name}-fields.csv`;
    a.click();
  };
  return (
    <div className="pcb-tab schx-fields">
      <div className="schx-fields-bar">
        <label className="pcb-search">
          <Icon name="search" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Filter" />
        </label>
        <label className="sch-check">
          <input type="checkbox" checked={group} onChange={(e) => setGroup(e.target.checked)} /> Group alike
        </label>
      </div>
      <div className="schx-grid-wrap">
        <table className="schx-grid">
          <thead>
            <tr>
              <th>Qty</th>
              {cols.map((c) => (
                <th key={c}>
                  {c}
                  {custom.includes(c) && canEdit && (
                    <button type="button" className="schx-x" title={`Remove ${c} from every part`} onClick={() => void commit("pcb.schematic.fields", { schematic: v.id, remove: c }, `${c} removed`)}>
                      ×
                    </button>
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.key}>
                <td className="qty">{r.members.length}</td>
                {cols.map((c) => {
                  const val = c === "Reference" ? r.members.map((m) => m.reference).join(", ") : stdField(r.members[0], c);
                  const mixed = c !== "Reference" && r.members.some((m) => stdField(m, c) !== val);
                  if (c === "Reference" && r.members.length > 1)
                    return (
                      <td key={c} className="refs" onClick={() => props.onSelect(r.members[0])}>
                        {val}
                      </td>
                    );
                  if (c === "DNP")
                    return (
                      <td key={c}>
                        <input type="checkbox" checked={!!val} disabled={!canEdit} onChange={(e) => save(r.members, c, e.target.checked ? "yes" : "no")} />
                      </td>
                    );
                  return (
                    <td key={c}>
                      <input key={`${r.key}-${c}-${val}`} defaultValue={mixed ? "" : val} placeholder={mixed ? "(mixed)" : ""} disabled={!canEdit} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} onBlur={(e) => e.target.value !== (mixed ? "" : val) && save(r.members, c, e.target.value)} />
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="sch-props-row wrap">
        <button
          type="button"
          className="secondary"
          disabled={!canEdit}
          onClick={() => {
            const n = window.prompt("New field name (e.g. Tolerance, Voltage, Datasheet)");
            if (n?.trim()) setExtra([...extra, n.trim()]);
          }}
        >
          + Field
        </button>
        <button type="button" className="secondary" onClick={csv}>
          <Icon name="download" /> CSV
        </button>
      </div>
      <p className="small sch-hint">Edit a cell to change every part in the row. Fields that every new part should get live in Setup → Field templates.</p>
    </div>
  );
}

// ---- Setup ----------------------------------------------------------------------------

const ERC_CODES: [string, string][] = [
  ["unconnected_pin", "Pin not connected"],
  ["pin_conflict", "Pin conflict (matrix)"],
  ["power_not_driven", "Power input not driven"],
  ["pin_not_driven", "Input not driven"],
  ["single_pin_net", "Net with one pin"],
  ["label_unmatched", "Label used once"],
  ["multiple_net_names", "Several names on a net"],
  ["dangling_wire", "Wire end unconnected"],
  ["duplicate_reference", "Duplicate reference"],
  ["not_annotated", "Not annotated"],
  ["missing_footprint", "No footprint"],
  ["unknown_footprint", "Unknown footprint"],
  ["pin_map", "Pin map"],
  ["missing_unit", "Unit not placed"],
  ["no_connect_connected", "No-connect flag on a wired pin"],
  ["no_connect_dangling", "No-connect flag not on a pin"],
  ["hier_pin_unmatched", "Sheet pin without label"],
  ["hier_label_unmatched", "Hierarchical label without pin"],
  ["bus_unnamed", "Bus without a name"],
  ["bus_wire_conflict", "Wire straight onto a bus"],
  ["bus_entry_unlabeled", "Bus entry wire not named"],
  ["bus_member_unknown", "Not a member of the bus"],
  ["bus_width_mismatch", "Bus widths differ"],
  ["bus_label_on_wire", "Bus label on a wire"],
];

function Section(props: { title: string; children: React.ReactNode; open?: boolean }) {
  const [open, setOpen] = useState(!!props.open);
  return (
    <div className={`schx-sec ${open ? "open" : ""}`}>
      <button type="button" className="schx-sec-head" onClick={() => setOpen(!open)}>
        <Icon name={open ? "down" : "chevron"} /> {props.title}
      </button>
      {open && <div className="schx-sec-body">{props.children}</div>}
    </div>
  );
}

const LEVEL_NEXT: Record<string, string> = { ok: "warning", warning: "error", error: "ok" };

export function SetupPanel(props: { v: SchematicV; commit: Commit; canEdit: boolean; notify: (m: string) => void }) {
  const { v, commit, canEdit } = props;
  const s = v.setup;
  const [newClass, setNewClass] = useState("");
  const [ann, setAnn] = useState({ order: "rows", numbering: "continuous", start: "1", reset: false });
  const types = s.pin_types;
  const short: Record<string, string> = { input: "In", output: "Out", bidirectional: "Bi", tri_state: "3S", passive: "Pas", unspecified: "Uns", power_in: "PwI", power_out: "PwO", open_collector: "OC", open_emitter: "OE" };
  const key = (a: string, b: string) => {
    const ia = types.indexOf(a);
    const ib = types.indexOf(b);
    return ia <= ib ? `${a}/${b}` : `${b}/${a}`;
  };
  const updSch = (p: Record<string, unknown>, done?: string) => void commit("pcb.schematic.update", { target: v.id, ...p }, done);
  const title = v.title;
  return (
    <div className="pcb-tab schx-setup">
      <Section title="Title block & drawing sheet" open>
        <Field label="Title" value={title.title} disabled={!canEdit} onSave={(x) => updSch({ title: { ...title, title: x } })} />
        <Field label="Company" value={title.company} disabled={!canEdit} onSave={(x) => updSch({ title: { ...title, company: x } })} />
        <Field label="Revision" value={title.revision} disabled={!canEdit} onSave={(x) => updSch({ title: { ...title, revision: x } })} />
        <Field label="Date" value={title.date} disabled={!canEdit} onSave={(x) => updSch({ title: { ...title, date: x } })} />
        {[0, 1, 2].map((i) => (
          <Field key={i} label={`Comment ${i + 1}`} value={title.comments?.[i] ?? ""} disabled={!canEdit} onSave={(x) => {
            const c = [...(title.comments ?? [])];
            while (c.length <= i) c.push("");
            c[i] = x;
            updSch({ title: { ...title, comments: c } });
          }} />
        ))}
        <div className="sch-props-row wrap">
          <label className="sch-check">
            <input type="checkbox" checked={!s.drawing_sheet.no_title_block} disabled={!canEdit} onChange={(e) => updSch({ drawing_sheet: { ...s.drawing_sheet, no_title_block: !e.target.checked } })} /> Title block
          </label>
          <label className="sch-check">
            <input type="checkbox" checked={!s.drawing_sheet.no_zones} disabled={!canEdit} onChange={(e) => updSch({ drawing_sheet: { ...s.drawing_sheet, no_zones: !e.target.checked } })} /> Zone references
          </label>
        </div>
        <h4 className="sch-sub">Extra items on every sheet</h4>
        {(s.drawing_sheet.items ?? []).map((it, i) => (
          <div key={i} className="schx-row">
            <select value={it.kind} disabled={!canEdit} onChange={(e) => updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).map((x, j) => (j === i ? { ...x, kind: e.target.value } : x)) } })}>
              <option value="text">Text</option>
              <option value="line">Line</option>
              <option value="rect">Box</option>
            </select>
            <select value={it.corner || "rb"} disabled={!canEdit} onChange={(e) => updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).map((x, j) => (j === i ? { ...x, corner: e.target.value } : x)) } })}>
              {["rb", "lb", "rt", "lt"].map((c) => (
                <option key={c} value={c}>
                  {{ rb: "↘", lb: "↙", rt: "↗", lt: "↖" }[c]}
                </option>
              ))}
            </select>
            <input defaultValue={it.at.join(",")} title="Offset from the corner (mm)" disabled={!canEdit} onBlur={(e) => {
              const at = e.target.value.split(",").map(Number);
              if (at.length === 2 && at.every(Number.isFinite)) updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).map((x, j) => (j === i ? { ...x, at } : x)) } });
            }} />
            {it.kind === "text" ? (
              <input defaultValue={it.text ?? ""} placeholder="Text, ${TITLE}…" disabled={!canEdit} onBlur={(e) => updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).map((x, j) => (j === i ? { ...x, text: e.target.value } : x)) } })} />
            ) : (
              <input defaultValue={(it.end ?? it.at).join(",")} title="Other end (mm)" disabled={!canEdit} onBlur={(e) => {
                const end = e.target.value.split(",").map(Number);
                if (end.length === 2 && end.every(Number.isFinite)) updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).map((x, j) => (j === i ? { ...x, end } : x)) } });
              }} />
            )}
            <button type="button" className="schx-x" disabled={!canEdit} onClick={() => updSch({ drawing_sheet: { ...s.drawing_sheet, items: (s.drawing_sheet.items ?? []).filter((_, j) => j !== i) } })}>
              ×
            </button>
          </div>
        ))}
        <button type="button" className="secondary" disabled={!canEdit} onClick={() => updSch({ drawing_sheet: { ...s.drawing_sheet, items: [...(s.drawing_sheet.items ?? []), { kind: "text", corner: "rb", at: [4, 36], text: "Drawn by ${AUTHOR}" }] } })}>
          + Item
        </button>
      </Section>
      <Section title={`Net classes (${s.netclasses.length})`}>
        {s.netclasses.map((c) => (
          <div key={c.name} className="schx-card">
            <div className="schx-class-head">
              <span className="schx-dot" style={{ background: c.color ?? "#1b7f3d" }} />
              <strong>{c.name}</strong>
              <button type="button" className="schx-x" disabled={!canEdit} onClick={() => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, delete: true })}>
                ×
              </button>
            </div>
            <Swatches value={c.color} allowNone disabled={!canEdit} onPick={(color) => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, color })} />
            <Field label="Nets" value={(c.patterns ?? []).join(" ")} placeholder="USB_* VBUS" disabled={!canEdit} onSave={(x) => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, patterns: x.split(/[\s,]+/).filter(Boolean) })} />
            <div className="schx-pair">
              <Field label="Wire mm" value={c.wire_width ? String(c.wire_width) : ""} disabled={!canEdit} onSave={(x) => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, wire_width: Number(x) || 0 })} />
              <Field label="Track mm" value={c.track_width ? String(c.track_width) : ""} disabled={!canEdit} onSave={(x) => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, track_width: Number(x) || 0 })} />
              <Field label="Clear mm" value={c.clearance ? String(c.clearance) : ""} disabled={!canEdit} onSave={(x) => void commit("pcb.schematic.netclass", { schematic: v.id, name: c.name, clearance: Number(x) || 0 })} />
            </div>
            <p className="small">{v.nets.filter((n) => n.class === c.name).map((n) => n.name).join(", ") || "No nets yet: name patterns, or put a {class} directive label on a wire."}</p>
          </div>
        ))}
        <form
          className="schx-row"
          onSubmit={(e) => {
            e.preventDefault();
            if (newClass.trim()) void commit("pcb.schematic.netclass", { schematic: v.id, name: newClass.trim() }, `Net class ${newClass.trim()}`);
            setNewClass("");
          }}
        >
          <input value={newClass} onChange={(e) => setNewClass(e.target.value)} placeholder="New class: Power, USB, Fast…" disabled={!canEdit} />
          <button type="submit" className="secondary" disabled={!canEdit || !newClass.trim()}>
            Add
          </button>
        </form>
      </Section>
      <Section title={`Bus aliases (${Object.keys(s.bus_aliases).length})`}>
        {Object.entries(s.bus_aliases).map(([name, members]) => (
          <div key={name} className="schx-row">
            <strong className="schx-alias">{name}</strong>
            <input defaultValue={members.join(" ")} disabled={!canEdit} onBlur={(e) => {
              const m = e.target.value.split(/[\s,]+/).filter(Boolean);
              if (m.length) updSch({ bus_aliases: { ...s.bus_aliases, [name]: m } });
            }} />
            <button type="button" className="schx-x" disabled={!canEdit} onClick={() => {
              const a = { ...s.bus_aliases };
              delete a[name];
              updSch({ bus_aliases: a });
            }}>
              ×
            </button>
          </div>
        ))}
        <button type="button" className="secondary" disabled={!canEdit} onClick={() => {
          const name = window.prompt("Alias name (e.g. USB)");
          const members = name && window.prompt(`Members of ${name} (e.g. DP DM VBUS)`);
          if (name?.trim() && members?.trim()) updSch({ bus_aliases: { ...s.bus_aliases, [name.trim()]: members.split(/[\s,]+/).filter(Boolean) } });
        }}>
          + Alias
        </button>
        <p className="small sch-hint">Name a bus with the alias ({"{USB}"}, or P1{"{USB}"} for P1.DP …).</p>
      </Section>
      <Section title="Electrical rules: pin conflicts">
        <div className="schx-matrix-wrap">
          <table className="schx-matrix">
            <thead>
              <tr>
                <th />
                {types.map((t) => (
                  <th key={t} title={t.replace(/_/g, " ")}>
                    {short[t] ?? t}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {types.map((a, i) => (
                <tr key={a}>
                  <th title={a.replace(/_/g, " ")}>{short[a] ?? a}</th>
                  {types.map((b, j) => {
                    if (j < i) return <td key={b} className="blank" />;
                    const k = key(a, b);
                    const level = s.pin_matrix[k] ?? "ok";
                    const changed = s.erc.pin_matrix?.[k] !== undefined;
                    return (
                      <td key={b}>
                        <button type="button" className={`lvl ${level} ${changed ? "changed" : ""}`} disabled={!canEdit} title={`${a.replace(/_/g, " ")} with ${b.replace(/_/g, " ")}: ${level}${changed ? " (changed)" : ""}`} onClick={() => void commit("pcb.schematic.erc", { schematic: v.id, pin_matrix: { [k]: LEVEL_NEXT[level] } })}>
                          {level === "ok" ? "" : level === "warning" ? "W" : "E"}
                        </button>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="small sch-hint">Click a cell to cycle ok → warning → error. Outlined cells differ from the defaults.</p>
      </Section>
      <Section title="Electrical rules: severities">
        {ERC_CODES.map(([code, label]) => {
          const cur = s.erc.severities?.[code] ?? "default";
          return (
            <label key={code} className="sch-field schx-sev">
              <span>{label}</span>
              <select value={cur} disabled={!canEdit} onChange={(e) => void commit("pcb.schematic.erc", { schematic: v.id, severities: { [code]: e.target.value } })}>
                <option value="default">default</option>
                <option value="error">error</option>
                <option value="warning">warning</option>
                <option value="ignore">ignore</option>
              </select>
            </label>
          );
        })}
        {(s.erc.exclusions?.length ?? 0) > 0 && (
          <button type="button" className="secondary" disabled={!canEdit} onClick={() => void commit("pcb.schematic.erc", { schematic: v.id, include: (s.erc.exclusions ?? []).map((x) => x.key) }, "Exclusions cleared")}>
            Clear {s.erc.exclusions!.length} exclusion{s.erc.exclusions!.length === 1 ? "" : "s"}
          </button>
        )}
      </Section>
      <Section title={`Field templates (${s.field_templates.length})`}>
        {s.field_templates.map((t, i) => (
          <div key={t.name} className="schx-row">
            <strong className="schx-alias">{t.name}</strong>
            <input defaultValue={t.value ?? ""} placeholder="default value" disabled={!canEdit} onBlur={(e) => updSch({ field_templates: s.field_templates.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)) })} />
            <label className="sch-check" title="Show on the sheet">
              <input type="checkbox" checked={!!t.visible} disabled={!canEdit} onChange={(e) => updSch({ field_templates: s.field_templates.map((x, j) => (j === i ? { ...x, visible: e.target.checked } : x)) })} />
              <Icon name="eye" />
            </label>
            <button type="button" className="schx-x" disabled={!canEdit} onClick={() => updSch({ field_templates: s.field_templates.filter((_, j) => j !== i) })}>
              ×
            </button>
          </div>
        ))}
        <button type="button" className="secondary" disabled={!canEdit} onClick={() => {
          const n = window.prompt("Field every new part gets (e.g. Tolerance)");
          if (n?.trim()) updSch({ field_templates: [...s.field_templates, { name: n.trim(), value: "", visible: false }] });
        }}>
          + Template
        </button>
      </Section>
      <Section title={`Text variables (${Object.keys(s.text_vars).length})`}>
        {Object.entries(s.text_vars).map(([k, val]) => (
          <div key={k} className="schx-row">
            <strong className="schx-alias">{"${" + k + "}"}</strong>
            <input defaultValue={val} disabled={!canEdit} onBlur={(e) => updSch({ text_vars: { ...s.text_vars, [k]: e.target.value } })} />
            <button type="button" className="schx-x" disabled={!canEdit} onClick={() => {
              const t = { ...s.text_vars };
              delete t[k];
              updSch({ text_vars: t });
            }}>
              ×
            </button>
          </div>
        ))}
        <button type="button" className="secondary" disabled={!canEdit} onClick={() => {
          const n = window.prompt("Variable name (e.g. AUTHOR)");
          if (n?.trim()) updSch({ text_vars: { ...s.text_vars, [n.trim().toUpperCase()]: "" } });
        }}>
          + Variable
        </button>
      </Section>
      <Section title="Annotation">
        <label className="sch-field">
          <span>Order</span>
          <select value={ann.order} onChange={(e) => setAnn({ ...ann, order: e.target.value })}>
            <option value="rows">Across, then down</option>
            <option value="columns">Down, then across</option>
          </select>
        </label>
        <label className="sch-field">
          <span>Numbers</span>
          <select value={ann.numbering} onChange={(e) => setAnn({ ...ann, numbering: e.target.value })}>
            <option value="continuous">Continuous</option>
            <option value="sheet">Page × 100</option>
            <option value="sheet1000">Page × 1000</option>
          </select>
        </label>
        {ann.numbering === "continuous" && <Field label="Start at" value={ann.start} onSave={(x) => setAnn({ ...ann, start: x })} />}
        <label className="sch-check">
          <input type="checkbox" checked={ann.reset} onChange={(e) => setAnn({ ...ann, reset: e.target.checked })} /> Renumber everything (not only R?)
        </label>
        <button
          type="button"
          className="primary"
          disabled={!canEdit}
          onClick={async () => {
            const out = (await commit("pcb.schematic.annotate", { schematic: v.id, order: ann.order, numbering: ann.numbering, start: Number(ann.start) || 1, reset: ann.reset })) as { output?: { renamed: unknown[] } } | null;
            if (out?.output) props.notify(`${out.output.renamed.length} reference${out.output.renamed.length === 1 ? "" : "s"} numbered`);
          }}
        >
          Annotate
        </button>
      </Section>
    </div>
  );
}

// ---- Exports --------------------------------------------------------------------------

export function ExportMenu(props: { v: SchematicV; sheet: string; onClose: () => void }) {
  const { v } = props;
  const [group, setGroup] = useState("Value,Footprint,MPN");
  const [fields, setFields] = useState("Reference,Qty,Value,Footprint,MPN,Manufacturer,LCSC,Description");
  const [dnp, setDnp] = useState(false);
  const url = (path: string, extra: Record<string, string> = {}) => {
    const u = new URL(projectUrl(path), window.location.href);
    for (const [k, x] of Object.entries(extra)) u.searchParams.set(k, x);
    u.searchParams.set("download", "1");
    return u.toString();
  };
  const custom = [...new Set(v.symbols.flatMap((s) => Object.keys(s.fields ?? {})))];
  return (
    <div className="schx-menu" onPointerDown={(e) => e.stopPropagation()}>
      <div className="schx-menu-head">
        <strong>Export</strong>
        <button type="button" className="schx-x" onClick={props.onClose}>
          ×
        </button>
      </div>
      <h4 className="sch-sub">Drawings</h4>
      <div className="schx-links">
        <a href={url(`/pcb/schematic/${v.id}/schematic.pdf`)}>PDF, every sheet</a>
        <a href={url(`/pcb/schematic/${v.id}/sheet/${encodeURIComponent(props.sheet)}.svg`)}>SVG, this sheet</a>
      </div>
      <h4 className="sch-sub">Netlist</h4>
      <div className="schx-links">
        <a href={url(`/pcb/schematic/${v.id}/netlist`, { format: "kicad" })}>KiCad .net</a>
        <a href={url(`/pcb/schematic/${v.id}/netlist`, { format: "orcad" })}>OrCAD PCB2</a>
        <a href={url(`/pcb/schematic/${v.id}/netlist`, { format: "cadstar" })}>CADSTAR</a>
        <a href={url(`/pcb/schematic/${v.id}/netlist`, { format: "spice" })}>SPICE</a>
      </div>
      <h4 className="sch-sub">Bill of materials</h4>
      <label className="sch-field">
        <span>Group by</span>
        <input value={group} onChange={(e) => setGroup(e.target.value)} />
      </label>
      <label className="sch-field">
        <span>Columns</span>
        <input value={fields} onChange={(e) => setFields(e.target.value)} />
      </label>
      {custom.length > 0 && <p className="small">Your fields: {custom.join(", ")}</p>}
      <label className="sch-check">
        <input type="checkbox" checked={dnp} onChange={(e) => setDnp(e.target.checked)} /> Include do-not-populate parts
      </label>
      <a className="primary schx-btn" href={url(`/pcb/schematic/${v.id}/bom`, { group, fields, ...(dnp ? { dnp: "1" } : {}) })}>
        <Icon name="download" /> BOM CSV
      </a>
    </div>
  );
}

// ---- Find and replace ----------------------------------------------------------------

export type Hit = { kind: "symbol" | "label" | "text" | "net"; id: string; sheet: string; text: string; at: [number, number]; sel: Sel };

export function FindBar(props: { v: SchematicV; commit: Commit; canEdit: boolean; onGo: (h: Hit) => void; onClose: () => void; net: (n: string) => void; notify: (m: string) => void }) {
  const { v } = props;
  const [find, setFind] = useState("");
  const [replace, setReplace] = useState("");
  const [showReplace, setShowReplace] = useState(false);
  const [caseOn, setCase] = useState(false);
  const [word, setWord] = useState(false);
  const [wild, setWild] = useState(false);
  const [k, setK] = useState(0);
  const match = (s: string) => {
    if (!find) return false;
    if (wild && /[*?]/.test(find)) {
      const re = new RegExp(`^${find.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`, caseOn ? "" : "i");
      return re.test(s);
    }
    const a = caseOn ? s : s.toLowerCase();
    const b = caseOn ? find : find.toLowerCase();
    if (!word) return a.includes(b);
    return new RegExp(`(^|[^\\w])${b.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^\\w]|$)`).test(a);
  };
  const hits: Hit[] = useMemo(() => {
    const out: Hit[] = [];
    for (const s of v.symbols) if ([s.reference, s.value, s.footprint, s.mpn ?? "", ...Object.values(s.fields ?? {})].some(match)) out.push({ kind: "symbol", id: s.id, sheet: s.sheet, text: `${s.reference} ${s.value}`, at: [(s.bbox[0] + s.bbox[2]) / 2, (s.bbox[1] + s.bbox[3]) / 2], sel: { kind: "symbol", id: s.id } });
    for (const l of v.labels) if (match(l.text)) out.push({ kind: "label", id: l.id, sheet: l.sheet, text: l.text, at: l.at, sel: { kind: "label", id: l.id } });
    for (const t of v.texts) if (match(t.text)) out.push({ kind: "text", id: t.id, sheet: t.sheet, text: t.text.split("\n")[0], at: t.at, sel: { kind: "text", id: t.id } });
    for (const n of v.nets) if (match(n.name)) out.push({ kind: "net", id: n.name, sheet: "", text: n.name, at: [0, 0], sel: null });
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [v, find, caseOn, word, wild]);
  const go = (i: number) => {
    if (!hits.length) return;
    const j = ((i % hits.length) + hits.length) % hits.length;
    setK(j);
    const h = hits[j];
    if (h.kind === "net") props.net(h.id);
    else props.onGo(h);
  };
  return (
    <div className="schx-find" onPointerDown={(e) => e.stopPropagation()}>
      <div className="schx-find-row">
        <Icon name="search" />
        <input autoFocus value={find} placeholder="Find references, values, labels, notes, nets" onChange={(e) => (setFind(e.target.value), setK(0))} onKeyDown={(e) => {
          if (e.key === "Enter") go(e.shiftKey ? k - 1 : k + (hits[k] ? 1 : 0));
          if (e.key === "Escape") props.onClose();
        }} />
        <span className="schx-count">{find ? `${hits.length ? k + 1 : 0}/${hits.length}` : ""}</span>
        <button type="button" className={caseOn ? "on" : ""} title="Match case" onClick={() => setCase(!caseOn)}>
          Aa
        </button>
        <button type="button" className={word ? "on" : ""} title="Whole words" onClick={() => setWord(!word)}>
          ab
        </button>
        <button type="button" className={wild ? "on" : ""} title="Wildcards * ?" onClick={() => setWild(!wild)}>
          *
        </button>
        <button type="button" title="Previous (Shift+Enter)" onClick={() => go(k - 1)}>
          ↑
        </button>
        <button type="button" title="Next (Enter)" onClick={() => go(k + 1)}>
          ↓
        </button>
        <button type="button" className={showReplace ? "on" : ""} title="Replace" onClick={() => setShowReplace(!showReplace)} disabled={!props.canEdit}>
          ⇄
        </button>
        <button type="button" title="Close (Esc)" onClick={props.onClose}>
          ×
        </button>
      </div>
      {showReplace && (
        <div className="schx-find-row">
          <Icon name="pencil" />
          <input value={replace} placeholder="Replace with" onChange={(e) => setReplace(e.target.value)} />
          <button
            type="button"
            disabled={!hits[k] || hits[k].kind === "net"}
            onClick={async () => {
              const h = hits[k];
              await props.commit("pcb.schematic.replace", { schematic: v.id, find, replace, match_case: caseOn, whole_word: word, wildcards: wild, in: ["values", "fields", "labels", "texts", "references"], targets: [h.id] });
            }}
          >
            Replace
          </button>
          <button
            type="button"
            disabled={!hits.length}
            onClick={async () => {
              const out = (await props.commit("pcb.schematic.replace", { schematic: v.id, find, replace, match_case: caseOn, whole_word: word, wildcards: wild, in: ["values", "fields", "labels", "texts"] })) as { output?: { count: number } } | null;
              if (out?.output) props.notify(`${out.output.count} replaced`);
            }}
          >
            All
          </button>
        </div>
      )}
      {find && hits.length > 0 && (
        <div className="schx-hits">
          {hits.slice(0, 40).map((h, i) => (
            <button key={`${h.kind}${h.id}${h.sheet}`} type="button" className={i === k ? "active" : ""} onClick={() => go(i)}>
              <span className="kind">{h.kind}</span> {h.text}
              {h.sheet && v.sheets.length > 1 ? <small> · {v.sheets.find((s) => s.id === h.sheet)?.name}</small> : null}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---- ERC with exclusions ---------------------------------------------------------------

export function ErcPanel(props: { v: SchematicV; onFocus: (x: Erc) => void; commit: Commit; canEdit: boolean }) {
  const [warn, setWarn] = useState(true);
  const [showExcluded, setShowExcluded] = useState(false);
  const { v } = props;
  const list = v.violations.filter((x) => (warn || x.severity === "error") && (showExcluded || !x.excluded));
  const errors = v.stats.errors;
  const excluded = v.violations.filter((x) => x.excluded).length;
  return (
    <div className="pcb-tab">
      <div className={`sch-erc-head ${errors ? "bad" : "good"}`}>
        <Icon name={errors ? "alert" : "check"} />
        <div>
          <strong>{errors ? `${errors} electrical ${errors === 1 ? "error" : "errors"}` : "Electrical rules pass"}</strong>
          <span>
            {v.stats.parts} parts · {v.stats.nets} nets · {v.stats.warnings} warning{v.stats.warnings === 1 ? "" : "s"}
            {excluded ? ` · ${excluded} excluded` : ""}
          </span>
        </div>
      </div>
      <div className="sch-props-row wrap">
        <label className="sch-check">
          <input type="checkbox" checked={warn} onChange={(e) => setWarn(e.target.checked)} /> Warnings
        </label>
        {excluded > 0 && (
          <label className="sch-check">
            <input type="checkbox" checked={showExcluded} onChange={(e) => setShowExcluded(e.target.checked)} /> Excluded
          </label>
        )}
      </div>
      <div className="pcb-list sch-list">
        {list.map((x, i) => (
          <div key={`${x.key}${i}`} className={`schx-erc ${x.excluded ? "excluded" : ""}`}>
            <button type="button" className="pcb-row sch-erc" onClick={() => props.onFocus(x)}>
              <span className={`sch-sev ${x.severity}`} />
              <span className="val">
                {x.message}
                <small>
                  {x.code.replace(/_/g, " ")}
                  {x.excluded ? " · excluded" : ""}
                </small>
              </span>
            </button>
            <button
              type="button"
              className="schx-exclude"
              disabled={!props.canEdit}
              title={x.excluded ? "Count it again" : "Exclude: reviewed, it is intended"}
              onClick={() => void props.commit("pcb.schematic.erc", x.excluded ? { schematic: v.id, include: [x.key] } : { schematic: v.id, exclude: [{ key: x.key }] }, x.excluded ? "Counted again" : "Excluded")}
            >
              {x.excluded ? "Include" : "Exclude"}
            </button>
          </div>
        ))}
        {!list.length && <p className="small">Nothing to fix.</p>}
      </div>
    </div>
  );
}
