// Data: spreadsheets, parameter sets and configuration tables, in and out.
// A sheet is a grid of numbers (with units), text and formulas; a cell's
// alias is read anywhere in the project as `Sheet.alias` (link a parameter
// to it and the model follows the sheet). Tables come in from CSV and Excel
// files and go out the same way.

import { useEffect, useMemo, useRef, useState } from "react";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import "./data.css";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type SheetData = { name: string; description?: string; cells: Record<string, { input?: string; alias?: string; unit?: string }> };

const sheetOf = (e: EntityView | undefined) => e?.components["shape.sheet"] as SheetData | undefined;
const setOf = (e: EntityView | undefined) => (e && !e.components["draft.array"] ? (e.components["shape.parameter_set"] as { name: string; params: { key: string }[] } | undefined) : undefined);
const configsOf = (e: EntityView | undefined) => e?.components["shape.configurations"] as { params: string[]; rows: { name: string }[] } | undefined;

export function colName(c: number) {
  let s = "";
  c += 1;
  while (c > 0) {
    const r = (c - 1) % 26;
    s = String.fromCharCode(65 + r) + s;
    c = Math.floor((c - 1) / 26);
  }
  return s;
}

function parseAddr(a: string): [number, number] | null {
  const m = /^([A-Z]+)(\d+)$/.exec(a);
  if (!m) return null;
  let c = 0;
  for (const ch of m[1]) c = c * 26 + ch.charCodeAt(0) - 64;
  return [c - 1, Number(m[2]) - 1];
}

const UNITS = ["", "mm", "cm", "m", "in", "ft", "deg"];

/** Reads a file for an import command: text for CSV, base64 for a workbook. */
async function filePayload(f: File): Promise<Record<string, unknown>> {
  const xlsx = /\.xlsx?m?$/i.test(f.name);
  if (!xlsx) return { format: "csv", text: await f.text() };
  const bytes = new Uint8Array(await f.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return { format: "xlsx", base64: btoa(bin) };
}

function FilePick(props: { label: string; icon?: "upload"; onFile: (f: File) => void; className?: string }) {
  return (
    <label className={`secondary file-pick ${props.className ?? ""}`}>
      <Icon name={props.icon ?? "upload"} />
      {props.label}
      <input
        type="file"
        accept=".csv,.tsv,.txt,.xlsx,.xlsm"
        aria-label={props.label}
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) props.onFile(f);
          e.target.value = "";
        }}
      />
    </label>
  );
}

export function SheetWorkspace(props: { entities: ReadonlyMap<string, EntityView>; run: Run; notify: (t: string) => void; onClose: () => void; open?: string | null }) {
  const { entities, run, notify } = props;
  const all = [...entities.values()];
  const sheets = all.filter((e) => sheetOf(e));
  const sets = all.filter((e) => setOf(e));
  const tables = all.filter((e) => configsOf(e) && e.components["shape.program"]);
  const [current, setCurrent] = useState<string | null>(props.open ?? sheets[0]?.id ?? null);
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  useEffect(() => {
    if (current && !entities.has(current)) setCurrent(sheets[0]?.id ?? null);
  }, [entities, current, sheets]);

  const newSheet = async () => {
    const used = new Set(sheets.map((e) => sheetOf(e)!.name));
    let n = 1;
    while (used.has(`Sheet ${n}`)) n++;
    const out = (await run("sheet.create", { name: `Sheet ${n}`, cells: { A1: "Name", B1: "Value" } })) as { output: { id: string } } | null;
    if (out) setCurrent(out.output.id);
  };
  const importSheet = async (f: File, target?: string) => {
    const name = f.name.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ").replace(/\./g, " ").trim() || "Imported";
    const out = (await run("sheet.import", { ...(await filePayload(f)), ...(target ? { target, replace: true } : { name }) })) as { output: { id: string; notes?: string[] } } | null;
    if (!out) return;
    setCurrent(out.output.id);
    notify(out.output.notes?.length ? `Imported ${f.name}: ${out.output.notes.length} formulas kept as values (${out.output.notes[0]})` : `Imported ${f.name}`);
  };

  return (
    <dialog ref={ref} className="data-workspace" aria-label="Data: sheets and tables" onClose={props.onClose} onCancel={(e) => (e.preventDefault(), props.onClose())}>
      <header className="data-head">
        <Icon name="table" />
        <h2>Data</h2>
        <span className="muted">Sheets calculate; link any parameter to a cell (=Sheet.alias) and the model follows.</span>
        <IconButton icon="close" label="Close data" onClick={props.onClose} />
      </header>
      <div className="data-body">
        <aside className="data-side">
          <h3 className="panel-section">Sheets</h3>
          {sheets.map((e) => (
            <button key={e.id} type="button" className={`list-row ${current === e.id ? "active" : ""}`} title={`Open the sheet ${sheetOf(e)!.name}`} onClick={() => setCurrent(e.id)}>
              <Icon name="table" />
              <span>
                {sheetOf(e)!.name}
                <small className="row-count">
                  {Object.keys(e.sheet?.cells ?? {}).length} cells{e.sheet?.errors ? ` · ${e.sheet.errors} errors` : ""}
                </small>
              </span>
            </button>
          ))}
          <div className="data-actions">
            <button type="button" className="secondary" title="Make an empty sheet for numbers, text and formulas" onClick={newSheet}>
              <Icon name="plus" /> New sheet
            </button>
            <FilePick label="Import CSV / Excel" onFile={(f) => void importSheet(f)} />
          </div>
          <h3 className="panel-section">Parameter sets</h3>
          {sets.length === 0 && <p className="muted small">None yet. Import a table of names and values as one.</p>}
          {sets.map((e) => (
            <div key={e.id} className="data-table-row">
              <span>
                {setOf(e)!.name}
                <small className="row-count">{setOf(e)!.params.length} values</small>
              </span>
              <a className="link-button" href={projectUrl(`/table/${e.id}.csv`)} download title={`Download the parameter set ${setOf(e)!.name} as a CSV table`}>
                CSV
              </a>
              <a className="link-button" href={projectUrl(`/table/${e.id}.xlsx`)} download title={`Download the parameter set ${setOf(e)!.name} as an Excel workbook`}>
                XLSX
              </a>
              <FilePick
                label="Update"
                className="compact"
                onFile={async (f) => {
                  const r = (await run("parameters.import", { target: e.id, ...(await filePayload(f)) })) as { output: { added: number; updated: number } } | null;
                  if (r) notify(`${setOf(e)!.name}: ${r.output.updated} updated, ${r.output.added} added`);
                }}
              />
            </div>
          ))}
          <FilePick
            label="New set from a table"
            onFile={async (f) => {
              const name = f.name.replace(/\.[^.]+$/, "").replace(/[_.-]+/g, " ").trim() || "Imported values";
              const r = (await run("parameters.import", { name, ...(await filePayload(f)) })) as { output: { added: number } } | null;
              if (r) notify(`Made the parameter set ${name} (${r.output.added} values)`);
            }}
          />
          {tables.length > 0 && <h3 className="panel-section">Configuration tables</h3>}
          {tables.map((e) => (
            <div key={e.id} className="data-table-row">
              <span>
                {(e.components["shape.program"] as { name: string }).name}
                <small className="row-count">{configsOf(e)!.rows.length} configurations</small>
              </span>
              <a className="link-button" href={projectUrl(`/table/${e.id}.csv`)} download title={`Download the configurations of ${(e.components["shape.program"] as { name: string }).name} as a CSV table`}>
                CSV
              </a>
              <a className="link-button" href={projectUrl(`/table/${e.id}.xlsx`)} download title={`Download the configurations of ${(e.components["shape.program"] as { name: string }).name} as an Excel workbook`}>
                XLSX
              </a>
              <FilePick
                label="Replace"
                className="compact"
                onFile={async (f) => {
                  const r = (await run("shape.config.import", { target: e.id, ...(await filePayload(f)) })) as { output: { configurations: number } } | null;
                  if (r) notify(`${r.output.configurations} configurations read`);
                }}
              />
            </div>
          ))}
        </aside>
        <main className="data-main">
          {current && entities.get(current) && sheetOf(entities.get(current)) ? (
            <SheetGrid key={current} entity={entities.get(current)!} run={run} notify={notify} onImport={(f) => void importSheet(f, current)} />
          ) : (
            <div className="data-empty">
              <Icon name="table" />
              <p>A sheet holds the numbers a design runs on: sizes, a load table, a clearance budget, a cut list.</p>
              <p className="muted">Type numbers with units (200 mm, 2 in, 45°), text, or formulas (=B2*2, =sum(B2:B9), =Desk.width/2). Name a cell and link any parameter to it.</p>
              <button type="button" className="primary" title="Make an empty sheet for numbers, text and formulas" onClick={newSheet}>
                <Icon name="plus" /> New sheet
              </button>
            </div>
          )}
        </main>
      </div>
    </dialog>
  );
}

function SheetGrid(props: { entity: EntityView; run: Run; notify: (t: string) => void; onImport: (f: File) => void }) {
  const { entity, run } = props;
  const sheet = sheetOf(entity)!;
  const values = entity.sheet?.cells ?? {};
  const [sel, setSel] = useState<[number, number]>([0, 0]);
  const [editing, setEditing] = useState<string | null>(null);
  const [title, setTitle] = useState(sheet.name);
  const [aliasDraft, setAliasDraft] = useState<string | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const editor = useRef<HTMLInputElement>(null);
  useEffect(() => setTitle(sheet.name), [sheet.name]);

  const used = Object.keys(sheet.cells).map(parseAddr).filter(Boolean) as [number, number][];
  const cols = Math.max(8, ...used.map(([c]) => c + 3));
  const rows = Math.max(24, ...used.map(([, r]) => r + 6));
  const addr = (c: number, r: number) => `${colName(c)}${r + 1}`;
  const at = addr(sel[0], sel[1]);
  const cell = sheet.cells[at];
  const shownInput = (a: string) => values[a]?.formula ?? sheet.cells[a]?.input ?? "";

  const set = async (cells: Record<string, string>, extra: Record<string, unknown> = {}) => {
    await run("sheet.set", { target: entity.id, cells, ...extra });
  };
  const move = (dc: number, dr: number) => {
    setSel(([c, r]) => [Math.max(0, Math.min(cols - 1, c + dc)), Math.max(0, Math.min(rows - 1, r + dr))]);
  };
  const commitEdit = async (text: string, then: [number, number] = [0, 1]) => {
    const a = editing;
    setEditing(null);
    if (a !== null && text !== shownInput(a)) await set({ [a]: text });
    move(...then);
    requestAnimationFrame(() => grid.current?.focus());
  };
  useEffect(() => {
    if (editing !== null) requestAnimationFrame(() => editor.current?.focus());
  }, [editing]);

  const aliasNow = aliasDraft ?? cell?.alias ?? "";
  const useText = cell?.alias ? `=${sheet.name}.${cell.alias}` : `=${sheet.name}.${at}`;

  return (
    <div className="sheet">
      <div className="sheet-title">
        <input
          className="sheet-name"
          value={title}
          aria-label="Sheet name"
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title.trim() !== sheet.name && void run("sheet.set", { target: entity.id, name: title.trim() })}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        <a className="link-button" href={projectUrl(`/table/${entity.id}.csv`)} download title={`Download ${sheet.name} as a CSV table`}>
          <Icon name="download" /> CSV
        </a>
        <a className="link-button" href={projectUrl(`/table/${entity.id}.xlsx`)} download title={`Download ${sheet.name} as an Excel workbook`}>
          <Icon name="download" /> Excel
        </a>
        <FilePick label="Replace from file" onFile={props.onImport} className="compact" />
        <IconButton icon="trash" label="Delete sheet" onClick={() => void run("core.entity.delete", { target: entity.id })} />
      </div>
      <div className="formula-bar">
        <span className="formula-at">{at}</span>
        <input
          key={`${at}:${shownInput(at)}`}
          className="formula-input"
          aria-label={`Contents of ${at}`}
          defaultValue={shownInput(at)}
          placeholder="number with unit, text, or =formula"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              const v = e.currentTarget.value;
              if (v !== shownInput(at)) void set({ [at]: v });
              grid.current?.focus();
            }
            e.stopPropagation();
          }}
        />
        <label className="formula-alias" title="A name for this cell: read it anywhere as Sheet.alias">
          alias
          <input
            value={aliasNow}
            aria-label={`Alias of ${at}`}
            placeholder="name"
            onChange={(e) => setAliasDraft(e.target.value.replace(/[^A-Za-z0-9_]/g, "_"))}
            onBlur={async () => {
              if (aliasDraft !== null && aliasDraft !== (cell?.alias ?? "")) await set({}, { aliases: { [at]: aliasDraft } });
              setAliasDraft(null);
            }}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
        </label>
        <select
          aria-label={`Unit of ${at}`}
          value={cell?.unit ?? ""}
          onChange={(e) => void set({}, { units: { [at]: e.target.value } })}
          title="The unit this cell shows in (and a bare number typed here is in)"
        >
          {UNITS.map((u) => (
            <option key={u} value={u}>
              {u || "unit"}
            </option>
          ))}
        </select>
      </div>
      {values[at]?.error && <p className="sheet-error">{at}: {values[at]!.error}</p>}
      <div
        className="sheet-grid"
        ref={grid}
        tabIndex={0}
        role="grid"
        aria-label={sheet.name}
        style={{ gridTemplateColumns: `44px repeat(${cols}, minmax(96px, 1fr))` }}
        onKeyDown={(e) => {
          if (editing !== null) return;
          e.stopPropagation();
          const k = e.key;
          if (k === "ArrowUp") move(0, -1);
          else if (k === "ArrowDown") move(0, 1);
          else if (k === "ArrowLeft") move(-1, 0);
          else if (k === "ArrowRight" || k === "Tab") move(1, 0);
          else if (k === "Enter" || k === "F2") setEditing(at);
          else if (k === "Delete" || k === "Backspace") {
            if (sheet.cells[at]?.input) void set({ [at]: "" });
          } else if (k.length === 1 && !e.metaKey && !e.ctrlKey) {
            setEditing(at);
            requestAnimationFrame(() => {
              if (editor.current) {
                editor.current.value = k;
                editor.current.setSelectionRange(1, 1);
              }
            });
            e.preventDefault();
            return;
          } else return;
          e.preventDefault();
        }}
      >
        <div className="sheet-corner" />
        {Array.from({ length: cols }, (_, c) => (
          <div key={c} className={`sheet-col ${c === sel[0] ? "on" : ""}`}>
            {colName(c)}
          </div>
        ))}
        {Array.from({ length: rows }, (_, r) => (
          <SheetRow
            key={r}
            r={r}
            cols={cols}
            sheet={sheet}
            values={values}
            sel={sel}
            editing={editing}
            editor={editor}
            shownInput={shownInput}
            onPick={(c) => {
              setSel([c, r]);
              setAliasDraft(null);
            }}
            onEdit={(a) => setEditing(a)}
            onCommit={commitEdit}
            onCancel={() => {
              setEditing(null);
              grid.current?.focus();
            }}
          />
        ))}
      </div>
      <p className="sheet-foot muted small">
        Link a parameter to {at}: <code>{useText}</code>
        <button type="button" className="link-button" title={`Copy ${useText}, to type into a parameter and link it to this cell`} onClick={() => void navigator.clipboard?.writeText(useText).then(() => props.notify(`Copied ${useText}`))}>
          <Icon name="copy" /> Copy
        </button>
        · Formulas: + − × ÷ ^, sum average count min max round sqrt sin cos… ranges B2:B9, other parameters Name.key; units 5mm 2in 45°.
      </p>
    </div>
  );
}

function SheetRow(props: {
  r: number;
  cols: number;
  sheet: SheetData;
  values: Record<string, { shown: string; value?: number; error?: string; formula?: string }>;
  sel: [number, number];
  editing: string | null;
  editor: React.RefObject<HTMLInputElement | null>;
  shownInput: (a: string) => string;
  onPick: (c: number) => void;
  onEdit: (a: string) => void;
  onCommit: (text: string, then?: [number, number]) => void;
  onCancel: () => void;
}) {
  const { r, sheet, sel } = props;
  const cells = useMemo(() => Array.from({ length: props.cols }, (_, c) => c), [props.cols]);
  return (
    <>
      <div className={`sheet-row-head ${r === sel[1] ? "on" : ""}`}>{r + 1}</div>
      {cells.map((c) => {
        const a = `${colName(c)}${r + 1}`;
        const v = props.values[a];
        const alias = sheet.cells[a]?.alias;
        const chosen = sel[0] === c && sel[1] === r;
        if (props.editing === a)
          return (
            <input
              key={a}
              ref={props.editor}
              className="sheet-cell editing"
              defaultValue={props.shownInput(a)}
              aria-label={`Edit ${a}`}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") props.onCommit(e.currentTarget.value, [0, 1]);
                else if (e.key === "Tab") {
                  e.preventDefault();
                  props.onCommit(e.currentTarget.value, [1, 0]);
                } else if (e.key === "Escape") props.onCancel();
              }}
              onBlur={(e) => props.editing === a && props.onCommit(e.currentTarget.value, [0, 0])}
            />
          );
        return (
          <div
            key={a}
            role="gridcell"
            aria-selected={chosen}
            className={`sheet-cell ${chosen ? "chosen" : ""} ${v?.value != null ? "num" : ""} ${v?.error ? "bad" : ""} ${v?.formula ? "fx" : ""}`}
            title={v?.error ?? (v?.formula ? `${v.formula}` : undefined)}
            onMouseDown={() => props.onPick(c)}
            onDoubleClick={() => props.onEdit(a)}
          >
            {alias && <span className="sheet-alias">{alias}</span>}
            {v?.shown ?? ""}
          </div>
        );
      })}
    </>
  );
}

