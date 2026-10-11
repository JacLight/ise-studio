// Configurations: named sets of parameter values for one shape ("M6×20",
// "M8×30"), kept as a table — rows are configurations, columns the parameters
// they set (and the finishes they keep). The switcher previews a row on hover
// and activates it on click; instances pick a row of their definition.

import { useEffect, useMemo, useRef, useState } from "react";
import type { EntityView } from "./engine";
import { Icon } from "./icons";
import { type ConfigRow, type Configurations, configurations, fmt, program } from "./model";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

const FINISH_SHORT: Record<string, string> = { fillet: "Fillet", chamfer: "Chamfer", push: "Push", shell: "Shell", draft: "Draft" };

/** A name not yet in the table: "Config 2", "Config 3"… */
function freshName(rows: ConfigRow[]) {
  for (let n = rows.length + 1; ; n++) {
    const name = `Config ${n}`;
    if (!rows.some((r) => r.name === name)) return name;
  }
}

/** Whether the shape's values differ from its active configuration's (edited since it was activated). */
export function configModified(e: EntityView, table: Configurations | undefined) {
  const row = table?.rows.find((r) => r.name === table.active);
  const prog = program(e);
  if (!row || !prog) return false;
  return table!.params.some((k) => {
    const p = prog.params.find((q) => q.key === k);
    return p && Math.abs((e.params?.[k] ?? p.value) - row.values[k]) > 1e-9;
  });
}

/** A table cell: a number typed in place, committed on Enter or leaving it, Escape puts it back. */
function Cell(props: { value: number; label: string; onCommit: (v: number) => void }) {
  const [text, setText] = useState<string | null>(null);
  const commit = () => {
    if (text === null) return;
    const v = Number(text.replace(",", "."));
    setText(null);
    if (text.trim() !== "" && Number.isFinite(v) && Math.abs(v - props.value) > 1e-12) props.onCommit(v);
  };
  return (
    <input
      className="config-cell num"
      inputMode="decimal"
      aria-label={props.label}
      value={text ?? fmt(props.value).replace(/,/g, "")}
      onFocus={(e) => e.target.select()}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setText(null);
          requestAnimationFrame(() => (e.target as HTMLInputElement).blur());
        }
      }}
    />
  );
}

/** A configuration's name, renamed in place. */
function NameCell(props: { value: string; taken: (name: string) => boolean; onRename: (name: string) => void }) {
  const [text, setText] = useState<string | null>(null);
  const clean = (text ?? "").trim();
  const bad = text !== null && clean !== props.value && (clean === "" || props.taken(clean));
  const commit = () => {
    if (text === null) return;
    setText(null);
    if (!bad && clean !== props.value) props.onRename(clean);
  };
  return (
    <input
      className={`config-cell name ${bad ? "bad" : ""}`}
      aria-label={`Name of ${props.value}`}
      title={bad ? (clean === "" ? "A configuration needs a name" : `“${clean}” is taken`) : undefined}
      value={text ?? props.value}
      maxLength={60}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setText(null);
          requestAnimationFrame(() => (e.target as HTMLInputElement).blur());
        }
      }}
    />
  );
}

/**
 * The Configurations section of a shape's properties: the switcher (hover to
 * preview, click to activate), and the table to edit rows, columns and cells.
 */
export function ConfigSection(props: {
  entity: EntityView;
  run: Run;
  /** Shows a configuration in the view without committing (null puts it back). */
  onPreview: (row: ConfigRow | null) => void;
  /** Instances showing each configuration, by name. */
  users: Record<string, number>;
  /** Section title (an instance shows its definition's table as “Component configurations”). */
  title?: string;
}) {
  const { entity, run } = props;
  const prog = program(entity)!;
  const table = configurations(entity);
  const [editing, setEditing] = useState(false);
  const previewing = useRef(false);
  // A preview never outlives the section.
  useEffect(() => () => void (previewing.current && props.onPreview(null)), []); // eslint-disable-line react-hooks/exhaustive-deps

  const linked = (key: string) => !!entity.formulas?.[key];
  const current = (key: string) => entity.params?.[key] ?? prog.params.find((p) => p.key === key)?.value ?? 0;
  const label = (key: string) => prog.params.find((p) => p.key === key)?.label ?? key;
  const unit = (key: string) => {
    const u = prog.params.find((p) => p.key === key)?.unit ?? "";
    return u === "count" ? "×" : u === "deg" ? "°" : u;
  };
  // Finishes a row can leave off, by the parameter that sizes them (one column per size).
  const finishKeys = useMemo(() => {
    const seen = new Map<string, string>();
    for (const f of prog.finishes ?? []) if (!seen.has(f.key)) seen.set(f.key, `${FINISH_SHORT[f.kind] ?? f.kind}`);
    return [...seen.entries()];
  }, [prog.finishes]);

  const save = (next: Configurations, renamed?: Record<string, string>) =>
    run("shape.config.set_table", { target: entity.id, params: next.params, rows: next.rows, active: next.active ?? "", ...(renamed ? { renamed } : {}) });

  const preview = (row: ConfigRow | null) => {
    previewing.current = !!row;
    props.onPreview(row);
  };
  const activate = async (name: string) => {
    preview(null);
    if (name !== table?.active) await run("shape.config.activate", { target: entity.id, name });
  };

  if (!table || table.rows.length === 0) {
    const plain = prog.params.filter((p) => !linked(p.key));
    return (
      <section className="props-section config-section">
        <header>
          <h3>{props.title ?? "Configurations"}</h3>
          <span className="section-note">Sizes or variants of this shape</span>
        </header>
        <button
          type="button"
          className="secondary finish-add config-start"
          disabled={!plain.length}
          title={plain.length ? "Start a table with this shape's values as the first configuration" : "Every parameter follows a formula; unlink one to configure it"}
          onClick={() => {
            const values = Object.fromEntries(plain.map((p) => [p.key, current(p.key)]));
            void save({ params: plain.map((p) => p.key), rows: [{ name: "Default", values }], active: "Default" });
            setEditing(true);
          }}
        >
          <Icon name="layers" /> Make configurations
        </button>
      </section>
    );
  }

  const modified = configModified(entity, table);
  const names = table.rows.map((r) => r.name);
  const segmented = names.length <= 4 && names.join("").length <= 26;
  const addable = prog.params.filter((p) => !table.params.includes(p.key));
  const withRow = (i: number, row: ConfigRow) => ({ ...table, rows: table.rows.map((r, j) => (j === i ? row : r)) });

  return (
    <section className="props-section config-section">
      <header>
        <h3>{props.title ?? "Configurations"}</h3>
        <span className="config-head-actions">
          {modified && (
            <span className="config-modified" title="The shape's values differ from this configuration's">
              Modified
            </span>
          )}
          <button type="button" className={`lock ${editing ? "active" : ""}`} aria-pressed={editing} onClick={() => setEditing((v) => !v)} title="Edit the configuration table">
            <Icon name="pencil" /> {editing ? "Done" : "Table"}
          </button>
        </span>
      </header>

      {segmented ? (
        <div className="seg config-switch" role="radiogroup" aria-label="Active configuration" onMouseLeave={() => previewing.current && preview(null)}>
          {table.rows.map((r) => (
            <button
              type="button"
              key={r.name}
              role="radio"
              aria-checked={r.name === table.active}
              className={r.name === table.active ? "active" : ""}
              title={r.description || table.params.map((k) => `${label(k)} ${fmt(r.values[k])}${unit(k)}`).join(" · ")}
              onMouseEnter={() => r.name !== table.active && preview(r)}
              onFocus={() => r.name !== table.active && preview(r)}
              onBlur={() => previewing.current && preview(null)}
              onClick={() => void activate(r.name)}
            >
              {r.name}
            </button>
          ))}
        </div>
      ) : (
        <select className="pop-select config-select" value={table.active ?? ""} aria-label="Active configuration" onChange={(e) => e.target.value && void activate(e.target.value)}>
          {!table.active && <option value="">None active</option>}
          {table.rows.map((r) => (
            <option key={r.name} value={r.name}>
              {r.name}
              {r.description ? ` — ${r.description}` : ""}
            </option>
          ))}
        </select>
      )}

      {modified && table.active && (
        <div className="config-modified-row">
          <button
            type="button"
            className="secondary"
            onClick={() => {
              const i = table.rows.findIndex((r) => r.name === table.active);
              const row = table.rows[i];
              void save(withRow(i, { ...row, values: Object.fromEntries(table.params.map((k) => [k, current(k)])) }));
            }}
            title={`Write the shape's current values into ${table.active}`}
          >
            <Icon name="check" /> Save to {table.active}
          </button>
          <button type="button" className="secondary" onClick={() => void run("shape.config.activate", { target: entity.id, name: table.active })} title={`Go back to ${table.active}'s values`}>
            <Icon name="undo" /> Revert
          </button>
        </div>
      )}

      {editing && (
        <div className="config-table-wrap">
          <table className="config-table">
            <thead>
              <tr>
                <th className="config-name-head">Name</th>
                {table.params.map((k) => (
                  <th key={k} title={`${label(k)} (${k})`}>
                    <span className="config-col">
                      <span className="config-col-label">
                        {label(k)}
                        {unit(k) && <small>{unit(k)}</small>}
                      </span>
                      {table.params.length > 1 && (
                        <button
                          type="button"
                          className="config-x"
                          aria-label={`Stop configuring ${label(k)}`}
                          title={`Stop configuring ${label(k)}: it keeps the shape's value`}
                          onClick={() =>
                            void save({
                              ...table,
                              params: table.params.filter((q) => q !== k),
                              rows: table.rows.map((r) => {
                                const { [k]: _, ...values } = r.values;
                                return { ...r, values };
                              }),
                            })
                          }
                        >
                          <Icon name="close" />
                        </button>
                      )}
                    </span>
                  </th>
                ))}
                {finishKeys.map(([key, kind]) => (
                  <th key={`f:${key}`} title={`Whether each configuration has the ${kind.toLowerCase()} sized by ${label(key)}`}>
                    <span className="config-col-label finish">{kind}</span>
                  </th>
                ))}
                <th aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {table.rows.map((r, i) => (
                <tr key={r.name} className={r.name === table.active ? "active" : ""}>
                  <td>
                    <NameCell
                      value={r.name}
                      taken={(n) => names.includes(n)}
                      onRename={(name) => void save({ ...withRow(i, { ...r, name }), active: table.active === r.name ? name : table.active }, { [r.name]: name })}
                    />
                  </td>
                  {table.params.map((k) => (
                    <td key={k}>
                      <Cell value={r.values[k] ?? current(k)} label={`${label(k)} in ${r.name}`} onCommit={(v) => void save(withRow(i, { ...r, values: { ...r.values, [k]: v } }))} />
                    </td>
                  ))}
                  {finishKeys.map(([key, kind]) => {
                    const on = !(r.suppress ?? []).includes(key);
                    return (
                      <td key={`f:${key}`} className="config-check">
                        <input
                          type="checkbox"
                          checked={on}
                          aria-label={`${kind} in ${r.name}`}
                          onChange={() => {
                            const suppress = on ? [...(r.suppress ?? []), key] : (r.suppress ?? []).filter((s) => s !== key);
                            void save(withRow(i, { ...r, suppress }));
                          }}
                        />
                      </td>
                    );
                  })}
                  <td className="config-row-actions">
                    <button
                      type="button"
                      className="config-x"
                      aria-label={`Delete ${r.name}`}
                      title={props.users[r.name] ? `${props.users[r.name]} instance${props.users[r.name] > 1 ? "s show" : " shows"} ${r.name}: switch them first` : `Delete ${r.name}`}
                      onClick={() => {
                        const rows = table.rows.filter((_, j) => j !== i);
                        void save(rows.length ? { ...table, rows, active: table.active === r.name ? null : table.active } : { params: [], rows: [], active: null });
                      }}
                    >
                      <Icon name="trash" />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {editing && (
        <div className="config-add-row">
          <button
            type="button"
            className="secondary finish-add"
            onClick={() => {
              const base = table.rows.find((r) => r.name === table.active) ?? table.rows[table.rows.length - 1];
              const values = Object.fromEntries(table.params.map((k) => [k, base?.values[k] ?? current(k)]));
              void save({ ...table, rows: [...table.rows, { name: freshName(table.rows), values, suppress: base?.suppress }] });
            }}
          >
            <Icon name="plus" /> Configuration
          </button>
          <select
            className="pop-select config-add-col"
            value=""
            aria-label="Configure another parameter"
            disabled={!addable.length}
            onChange={(e) => {
              const k = e.target.value;
              if (!k) return;
              void save({ ...table, params: [...table.params, k], rows: table.rows.map((r) => ({ ...r, values: { ...r.values, [k]: current(k) } })) });
            }}
          >
            <option value="">{addable.length ? "+ Parameter" : "All parameters in"}</option>
            {addable.map((p) => (
              <option key={p.key} value={p.key} disabled={linked(p.key)}>
                {p.label}
                {linked(p.key) ? " — follows a formula" : ""}
              </option>
            ))}
          </select>
        </div>
      )}
      {editing && addable.some((p) => linked(p.key)) && <p className="props-hint config-hint">Linked parameters follow their formulas, so a configuration can’t set them. Unlink one to configure it.</p>}
    </section>
  );
}

/** On an instance: which configuration of its shape it shows. */
export function InstanceConfig(props: { entity: EntityView; def: EntityView; config: string | null | undefined; run: Run }) {
  const table = configurations(props.def);
  if (!table?.rows.length) return null;
  return (
    <section className="props-section config-section">
      <header>
        <h3>Configuration</h3>
        <span className="section-note">{props.config ? "This instance only" : table.active ? `As defined · ${table.active}` : "As defined"}</span>
      </header>
      <select
        className="pop-select config-select"
        value={props.config ?? ""}
        aria-label="Configuration shown by this instance"
        onChange={(e) => void props.run("assembly.instance.update", { target: props.entity.id, config: e.target.value })}
      >
        <option value="">As defined{table.active ? ` (${table.active})` : ""}</option>
        {table.rows.map((r) => (
          <option key={r.name} value={r.name}>
            {r.name}
            {r.description ? ` — ${r.description}` : ""}
          </option>
        ))}
      </select>
    </section>
  );
}
