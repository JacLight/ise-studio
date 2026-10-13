// Components and assemblies in the panels: the tree of what is placed (with
// show, hide and isolate), the properties of an instance, and the stacking
// plan: every floor of a building with its use, sale status and owner.

import { useMemo, useState } from "react";
import { type EntityView, projectUrl } from "./engine";
import { Icon } from "./icons";
import {
  assemblyDef,
  assemblyParts,
  childrenOf,
  configurations,
  definitionName,
  fmt,
  formatMass,
  instance,
  isPart,
  partName,
  program,
  subPartKey,
  usesOf,
} from "./model";
import { AXIS_COLORS } from "./gizmo";
import { IMPORT_ACCEPT } from "./fileFormats";
import { ScrubField } from "./ScrubField";
import { LinkNote } from "./Panels";
import { ConfigSection, InstanceConfig } from "./Configurations";
import { statusColor } from "./Viewport";
import { AssemblyJoints, type JointView, jointName, jointOf } from "./Joints";
import { IDENTITY, worldOf } from "./model";
import { FitCheck } from "./FitCheck";
import { AssemblyStudio, PatternSection, isMate } from "./AssemblyTools";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

const STATUSES = ["Available", "Reserved", "Sold"];

const area = (e: EntityView | undefined) => e?.assembly?.metrics?.["Net area"] ?? null;
const keys = (e: EntityView | undefined) => e?.assembly?.metrics?.["Keys"] ?? null;
const fmtArea = (m2: number | null) => (m2 == null ? "" : `${Math.round(m2).toLocaleString("en-US")} m²`);

function StatusChip({ status }: { status?: string }) {
  if (!status) return null;
  return (
    <span className="status-chip" style={{ "--chip": statusColor(status) ?? "#9aa0ab" } as React.CSSProperties}>
      {status}
    </span>
  );
}

// ---- Tree ------------------------------------------------------------------

/** One piece of a part, or a group of pieces named "Group/Piece". */
type PieceNode = { name: string; path: string; children: PieceNode[] };

/** Nests a part's pieces by the "/" in their names, in the order they were made. */
export function pieceTree(names: string[]): PieceNode[] {
  const root: PieceNode = { name: "", path: "", children: [] };
  for (const full of names) {
    const segs = full.split("/").map((x) => x.trim()).filter(Boolean);
    let at = root;
    segs.forEach((seg, i) => {
      if (i === segs.length - 1) {
        at.children.push({ name: seg, path: full, children: [] });
        return;
      }
      const path = segs.slice(0, i + 1).join("/");
      let group = at.children.find((c) => c.path === path && c.path !== full);
      if (!group) {
        group = { name: seg, path, children: [] };
        at.children.push(group);
      }
      at = group;
    });
  }
  return root.children;
}

/** The pieces under a node (the node itself when it is one). */
const leaves = (n: PieceNode): string[] => (n.children.length ? n.children.flatMap(leaves) : [n.path]);

export function ComponentTree(props: {
  entities: ReadonlyMap<string, EntityView>;
  selected: string | null;
  hidden: ReadonlySet<string>;
  isolated: string | null;
  /** Shows only what matches (and the way down to it), opened. */
  query?: string;
  /** Picks an item, or one piece of it by name. */
  onSelect: (id: string, piece?: string) => void;
  onToggleHidden: (id: string) => void;
  onIsolate: (id: string | null) => void;
}) {
  const top = useMemo(() => [...props.entities.values()].filter(isPart), [props.entities]);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const q = (props.query ?? "").trim().toLowerCase();
  const hit = (name: string) => !q || name.toLowerCase().includes(q);
  // Open the way down to the selection, so picking in the view reveals it here.
  const openPath = useMemo(() => {
    const ids = new Set<string>();
    let current = props.selected ? props.entities.get(props.selected) : undefined;
    for (let i = 0; i < 32 && current; i++) {
      const parent = instance(current)?.parent;
      if (!parent) break;
      const owner = [...props.entities.values()].find((e) => instance(e)?.of === parent);
      if (!owner) break;
      ids.add(owner.id);
      current = owner;
    }
    return ids;
  }, [props.selected, props.entities]);

  const kidsOf = (e: EntityView) => {
    const inst = instance(e);
    return inst && assemblyDef(props.entities.get(inst.of)) ? childrenOf(inst.of, props.entities) : [];
  };
  // Whether an item, or anything inside it, matches the search (memoised per render).
  const memo = new Map<string, boolean>();
  const matches = (e: EntityView, depth = 0): boolean => {
    if (!q) return true;
    const known = memo.get(e.id);
    if (known !== undefined) return known;
    const own = hit(partName(e)) || (!instance(e) && assemblyParts(e).some((p) => hit(p.name)));
    const result = own || (depth < 32 && kidsOf(e).some((k) => matches(k, depth + 1)));
    memo.set(e.id, result);
    return result;
  };
  const toggle = (key: string) =>
    setOpen((o) => {
      const n = new Set(o);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const actions = (key: string, label: string) => {
    const off = props.hidden.has(key);
    const only = props.isolated === key;
    return (
      <span className="tree-actions">
        <button type="button" title={only ? "Show everything" : `Show only ${label}`} aria-label={only ? "Show everything" : `Show only ${label}`} className={only ? "on" : ""} onClick={() => props.onIsolate(only ? null : key)}>
          <Icon name="fit" />
        </button>
        <button type="button" title={off ? `Show ${label}` : `Hide ${label}`} aria-label={off ? `Show ${label}` : `Hide ${label}`} className={off ? "on" : ""} onClick={() => props.onToggleHidden(key)}>
          <Icon name="eye" />
        </button>
      </span>
    );
  };
  const twist = (key: string, expanded: boolean, can: boolean) =>
    can ? (
      <button type="button" className={`tree-twist ${expanded ? "open" : ""}`} aria-label={expanded ? "Collapse" : "Expand"} title={expanded ? "Collapse" : "Expand"} onClick={() => toggle(key)}>
        <Icon name="chevron" />
      </button>
    ) : (
      <span className="tree-twist" />
    );

  // Pieces of a part, nested by their groups. A group's eye hides every piece in it.
  const pieceRows = (e: EntityView, nodes: PieceNode[], depth: number, parentHidden: boolean): React.ReactNode[] =>
    nodes
      .filter((n) => !q || leaves(n).some((l) => hit(l)) || hit(n.name))
      .map((n) => {
        const key = subPartKey(e.id, n.path);
        const group = n.children.length > 0;
        const expanded = group && (open.has(key) || !!q);
        const off = group ? leaves(n).every((l) => props.hidden.has(subPartKey(e.id, l))) : props.hidden.has(key);
        return (
          <div key={key}>
            <div className={`tree-row ${off || parentHidden ? "is-hidden" : ""} ${q && hit(n.name) ? "is-match" : ""}`} style={{ paddingLeft: 6 + depth * 14 }}>
              {twist(key, expanded, group)}
              <button type="button" className="tree-name" title={n.path} onClick={() => (group ? toggle(key) : props.onSelect(e.id, n.path))}>
                <Icon name={group ? "layers" : "cube"} />
                <span>{n.name}</span>
                {group && <small>{leaves(n).length}</small>}
              </button>
              {group ? (
                <span className="tree-actions">
                  <button
                    type="button"
                    title={off ? `Show ${n.name}` : `Hide ${n.name}`}
                    aria-label={off ? `Show ${n.name}` : `Hide ${n.name}`}
                    className={off ? "on" : ""}
                    onClick={() => {
                      for (const l of leaves(n)) {
                        const k = subPartKey(e.id, l);
                        if (props.hidden.has(k) === off) props.onToggleHidden(k);
                      }
                    }}
                  >
                    <Icon name="eye" />
                  </button>
                </span>
              ) : (
                actions(key, n.name)
              )}
            </div>
            {expanded && pieceRows(e, n.children, depth + 1, off || parentHidden)}
          </div>
        );
      });

  const row = (e: EntityView, depth: number): React.ReactNode => {
    if (!matches(e)) return null;
    const inst = instance(e);
    const kids = kidsOf(e);
    // A part made of several pieces (shells, grip, trigger…) lists them, each with its own eye.
    const pieces = !inst ? assemblyParts(e) : [];
    const expandable = kids.length > 0 || pieces.length > 1;
    const expanded = open.has(e.id) || openPath.has(e.id) || (pieces.length > 1 && props.selected === e.id) || (!!q && expandable);
    const hidden = props.hidden.has(e.id);
    const shared = inst ? usesOf(inst.of, props.entities) : 0;
    return (
      <div key={e.id}>
        <div className={`tree-row ${props.selected === e.id ? "active" : ""} ${hidden ? "is-hidden" : ""} ${q && hit(partName(e)) ? "is-match" : ""}`} style={{ paddingLeft: 6 + depth * 14 }}>
          {twist(e.id, expanded, expandable)}
          <button type="button" className="tree-name" title={inst ? `${partName(e)} (${definitionName(inst.of, props.entities)})` : partName(e)} onClick={() => props.onSelect(e.id)}>
            <Icon name={expandable ? "layers" : "cube"} />
            <span>{partName(e)}</span>
            {inst && shared > 1 && <small title={`${definitionName(inst.of, props.entities)}, used ${shared} times`}>×{shared}</small>}
            <StatusChip status={inst?.properties?.status} />
          </button>
          {actions(e.id, partName(e))}
        </div>
        {expanded && kids.map((k) => row(k, depth + 1))}
        {expanded && pieces.length > 1 && pieceRows(e, pieceTree(pieces.map((p) => p.name)), depth + 1, hidden)}
      </div>
    );
  };

  const rows = top.map((e) => row(e, 0)).filter(Boolean);
  return (
    <div className="component-tree">
      {props.isolated && (
        <button type="button" className="isolate-banner" onClick={() => props.onIsolate(null)}>
          Showing only {props.isolated.includes("#") ? props.isolated.slice(props.isolated.indexOf("#") + 1) : partName(props.entities.get(props.isolated) ?? ({ components: {} } as EntityView))} · Show everything
        </button>
      )}
      {rows}
      {q && rows.length === 0 && <p className="muted small">Nothing named “{props.query}”.</p>}
    </div>
  );
}

/** The Components panel: search over the tree, then the tree; adding parts at the foot. */
export function ComponentsBody(props: { query: string; onQuery: (q: string) => void; empty: boolean; onAdd: () => void; onImport?: (file: File) => void; children: React.ReactNode }) {
  return (
    <div className="components-panel">
      <label className="component-search">
        <Icon name="search" />
        <input type="search" placeholder="Search components" aria-label="Search components" value={props.query} onChange={(e) => props.onQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && props.onQuery("")} />
      </label>
      {props.empty ? <p className="muted">No parts yet.</p> : props.children}
      <div className="components-foot">
        {props.onImport && (
          <label className="secondary add-part import-step" title="A supplier's part or another tool's model (STEP, IGES, STL, OBJ, PLY, glTF, 3MF), or a DXF/SVG drawing as a sketch">
            <Icon name="download" /> Import file
            <input
              type="file"
              accept={IMPORT_ACCEPT}
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) props.onImport!(f);
                e.target.value = "";
              }}
            />
          </label>
        )}
        <button type="button" className="secondary add-part" onClick={props.onAdd} title="Add a box part to start shaping">
          <Icon name="plus" /> Add box part
        </button>
      </div>
    </div>
  );
}

// ---- Stacking plan -----------------------------------------------------------

/** The building whose floors carry sale facts: the top-level instance with the most such children. */
export function stackingSubject(entities: ReadonlyMap<string, EntityView>): EntityView | null {
  let best: EntityView | null = null;
  let most = 0;
  for (const e of entities.values()) {
    const inst = instance(e);
    if (!inst || inst.parent) continue;
    const n = childrenOf(inst.of, entities).filter((c) => instance(c)?.properties?.status || instance(c)?.properties?.use).length;
    if (n > most) {
      most = n;
      best = e;
    }
  }
  return best;
}

export function StackingPlan(props: {
  entities: ReadonlyMap<string, EntityView>;
  building: EntityView;
  selected: string | null;
  colorBy: "material" | "status";
  onColorBy: (c: "material" | "status") => void;
  onSelect: (id: string) => void;
}) {
  const inst = instance(props.building)!;
  const floors = childrenOf(inst.of, props.entities).reverse(); // top floor first
  const totals = new Map<string, { floors: number; area: number }>();
  for (const f of floors) {
    const status = instance(f)?.properties?.status ?? "Podium and roof";
    const span = Math.max(1, childrenOf(instance(f)!.of, props.entities).filter((c) => area(c) != null).length);
    const t = totals.get(status) ?? { floors: 0, area: 0 };
    t.floors += assemblyDef(props.entities.get(instance(f)!.of)) ? span : 1;
    t.area += area(f) ?? 0;
    totals.set(status, t);
  }
  return (
    <div className="stacking">
      <div className="stacking-head">
        <strong>{partName(props.building)}</strong>
        <label className="color-toggle">
          <input type="checkbox" checked={props.colorBy === "status"} onChange={(e) => props.onColorBy(e.target.checked ? "status" : "material")} />
          Paint the building by status
        </label>
      </div>
      <div className="stacking-totals">
        {[...totals].map(([status, t]) => (
          <div key={status}>
            {status === "Podium and roof" ? <span className="status-chip plain">{status}</span> : <StatusChip status={status} />}
            <span>
              {t.floors} {t.floors === 1 ? "floor" : "floors"} · {fmtArea(t.area)}
            </span>
          </div>
        ))}
      </div>
      <div className="stacking-rows">
        {floors.map((f) => {
          const fi = instance(f)!;
          const group = assemblyDef(props.entities.get(fi.of));
          const p = fi.properties ?? {};
          return (
            <button type="button" key={f.id} className={`stack-row ${props.selected === f.id ? "active" : ""} ${group ? "group" : ""}`} onClick={() => props.onSelect(f.id)}>
              <span className="stack-bar" style={{ background: statusColor(p.status) ?? "#d8d5e0" }} />
              <span className="stack-main">
                <strong>{fi.name}</strong>
                <small>
                  {[p.use ?? definitionName(fi.of, props.entities), p.owner].filter(Boolean).join(" · ")}
                  {group && ` · ${childrenOf(fi.of, props.entities).length} linked floors`}
                </small>
              </span>
              <span className="stack-facts">
                <StatusChip status={p.status} />
                <small>
                  {fmtArea(area(f))}
                  {keys(f) ? ` · ${keys(f)} keys` : ""}
                </small>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ---- Instance properties -----------------------------------------------------

export function InstanceBody(props: { entity: EntityView; entities: ReadonlyMap<string, EntityView>; run: Run; onSelect: (id: string) => void; joints: JointView }) {
  const { entity, entities, run } = props;
  const inst = instance(entity)!;
  const def = entities.get(inst.of);
  const prog = def ? program(def) : undefined;
  const uses = usesOf(inst.of, entities);
  const kids = assemblyDef(def) ? childrenOf(inst.of, entities) : [];
  const stats = entity.assembly;
  const [draft, setDraft] = useState<Record<string, string>>({});
  const props_ = inst.properties ?? {};
  const setProp = (key: string, value: string) => run("assembly.instance.update", { target: entity.id, properties: { [key]: value } });
  const move = (k: 0 | 1 | 2, v: number) => {
    const at = [...inst.at] as [number, number, number];
    at[k] = v;
    return run("assembly.instance.update", { target: entity.id, at });
  };
  const rotation = (Array.isArray(inst.rotation) ? inst.rotation : [0, 0, Number(inst.rotation) || 0]) as [number, number, number];
  const turn = (k: 0 | 1 | 2, v: number) => {
    const r = [...rotation] as [number, number, number];
    r[k] = v;
    return run("assembly.instance.update", { target: entity.id, rotation: r });
  };
  // A joint places it: where it is comes from the joint, and it says which.
  const placed = entity.placed;
  // What a mate holds moves by hand along what it leaves free: only joints (and locks) hold outright.
  const byJoint = placed?.by ? entities.get(placed.by) : undefined;
  const byKind = jointOf(byJoint)?.kind ?? "";
  const holder = byJoint && !(isMate(byKind) && byKind !== "lock") ? byJoint : undefined;
  const holderJoint = jointOf(holder);
  const holderOther = holderJoint ? [holderJoint.a, holderJoint.b].map((end) => (typeof end === "object" && "instance" in end ? end.instance : null)).find((x) => x !== entity.id) : undefined;
  const owner = inst.parent ? [...entities.values()].find((e) => instance(e)?.of === inst.parent) : undefined;
  // In a configuration, the columns it sets show its values (edited in the definition's table).
  const table = def ? configurations(def) : undefined;
  const row = inst.config ? table?.rows.find((r) => r.name === inst.config) : undefined;
  return (
    <div className="props">
      <div className="props-head">
        <span className="part-glyph" aria-hidden />
        <div className="props-title">
          <strong>{inst.name}</strong>
          <small>
            {assemblyDef(def) ? "Assembly" : "Component"} “{definitionName(inst.of, entities)}”{inst.config ? ` in ${inst.config}` : ""} · {uses > 1 ? `shared by ${uses}` : "unique"}
          </small>
        </div>
        {props_.status && <StatusChip status={props_.status} />}
      </div>

      {owner && (
        <button type="button" className="crumb" onClick={() => props.onSelect(owner.id)}>
          <Icon name="chevron" /> In {partName(owner)}
        </button>
      )}

      <section className="props-section">
        <header>
          <h3>Sales and use</h3>
        </header>
        <div className="status-picker" role="radiogroup" aria-label="Status">
          {STATUSES.map((s) => (
            <button type="button" key={s} role="radio" aria-checked={props_.status === s} className={props_.status === s ? "active" : ""} style={{ "--chip": statusColor(s) } as React.CSSProperties} onClick={() => setProp("status", props_.status === s ? "" : s)}>
              {s}
            </button>
          ))}
        </div>
        {(["use", "owner"] as const).map((key) => (
          <label key={key} className="prop-field">
            <span>{key === "use" ? "Use" : "Owner"}</span>
            <input
              value={draft[key] ?? props_[key] ?? ""}
              placeholder={key === "use" ? "Residence, Hotel, Serviced suites…" : "Buyer or operator"}
              onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))}
              onBlur={() => {
                if (draft[key] !== undefined && draft[key] !== (props_[key] ?? "")) setProp(key, draft[key].trim());
                setDraft((d) => {
                  const { [key]: _, ...rest } = d;
                  return rest;
                });
              }}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            />
          </label>
        ))}
      </section>

      {stats && (
        <div className="stats">
          <div>
            <span>{stats.metrics["Net area"] != null ? "Net area" : "Mass"}</span>
            <strong>{stats.metrics["Net area"] != null ? fmtArea(stats.metrics["Net area"]) : formatMass(stats.mass_kg)}</strong>
          </div>
          <div>
            <span>{stats.metrics["Keys"] != null ? "Hotel keys" : "Parts"}</span>
            <strong>{stats.metrics["Keys"] != null ? stats.metrics["Keys"] : stats.parts.toLocaleString()}</strong>
          </div>
        </div>
      )}

      <section className="props-section">
        <header>
          <h3>Position</h3>
          <span className="section-note">{inst.parent ? "In its assembly" : "In the project"}</span>
        </header>
        {holder && placed ? (
          <>
            <p className="held-note">
              <Icon name="link" />
              <span>
                Placed by the {holderJoint?.kind} joint <strong>{jointName(holder)}</strong>
                {holderOther !== undefined && <> to {holderOther ? (instance(entities.get(holderOther))?.name ?? "?") : "the ground"}</>}. Move it by the joint's value, or delete the joint to move it freely.
              </span>
            </p>
            <div className="held-readout">
              {(["X", "Y", "Z"] as const).map((axis, k) => (
                <span key={axis}>
                  <i style={{ color: AXIS_COLORS[k] }}>{axis}</i>
                  {fmt(placed.at[k])}
                  <small>mm</small>
                </span>
              ))}
              {(["X", "Y", "Z"] as const).map((axis, k) => (
                <span key={`r${axis}`}>
                  <i style={{ color: AXIS_COLORS[k] }}>∠{axis}</i>
                  {fmt(placed.rotation[k])}
                  <small>°</small>
                </span>
              ))}
            </div>
          </>
        ) : (
          <>
            <div className="scrub-grid">
              {(["X", "Y", "Z"] as const).map((axis, k) => (
                <ScrubField key={axis} label={`${axis} position`} chip={axis} tone={AXIS_COLORS[k]} value={inst.at[k]} onLive={() => {}} onCommit={(v) => move(k as 0 | 1 | 2, v)} />
              ))}
            </div>
            <div className="scrub-grid rotation-grid" title="Degrees about X, then Y, then Z (or drag a ring in the view)">
              {(["X", "Y", "Z"] as const).map((axis, k) => (
                <ScrubField key={axis} label={`Turn about ${axis}`} chip={`∠${axis}`} tone={AXIS_COLORS[k]} value={rotation[k]} unit="°" step={1} onLive={() => {}} onCommit={(v) => turn(k as 0 | 1 | 2, v)} />
              ))}
            </div>
          </>
        )}
      </section>

      <AssemblyJoints scope={inst.parent} focus={entity.id} frame={inst.parent ? worldOf(entity.id, entities, true) : IDENTITY} title="Joints" entities={entities} run={run} view={props.joints} />
      {assemblyDef(def) && (
        <AssemblyJoints scope={inst.of} frame={worldOf(entity.id, entities)} title={`Joints inside ${definitionName(inst.of, entities)}`} entities={entities} run={run} view={props.joints} />
      )}
      {assemblyDef(def) && <FitCheck instance={entity.id} onSelect={props.onSelect} />}
      {assemblyDef(def) && <AssemblyStudio scope={inst.of} frame={worldOf(entity.id, entities)} entities={entities} run={run} view={props.joints} onSelect={props.onSelect} />}
      <PatternSection entity={entity} entities={entities} run={run} onSelect={props.onSelect} />

      {prog && def && <InstanceConfig entity={entity} def={def} config={inst.config} run={run} />}

      {kids.length > 0 && (
        <section className="props-section">
          <header>
            <h3>Contents</h3>
            <span className="section-note">{kids.length} placed</span>
          </header>
          {kids.map((k) => (
            <button type="button" key={k.id} className="list-row" onClick={() => props.onSelect(k.id)}>
              <Icon name="cube" />
              <span>
                {instance(k)!.name}
                <small className="row-count">{definitionName(instance(k)!.of, entities)}</small>
              </span>
              <Icon name="chevron" />
            </button>
          ))}
        </section>
      )}

      {prog && def && (
        <section className="props-section">
          <header>
            <h3>Component parameters</h3>
            {uses > 1 && <span className="section-note warn">Changes all {uses}</span>}
          </header>
          {prog.description && <p className="shape-description">{prog.description}</p>}
          <div className="param-list">
            {prog.params.map((param) => (
              <div key={param.key} className={`param-row ${def.formulas?.[param.key] ? "linked" : ""}`}>
                <span className="param-label">
                  {param.label}
                  {def.formulas?.[param.key] && <LinkNote formula={def.formulas[param.key]} onUnlink={() => run("shape.link_param", { target: def.id, key: param.key, expr: null })} />}
                </span>
                {row && param.key in row.values ? (
                  <span className="config-fixed" title={`Set by ${row.name}: edit it in ${prog.name}'s configuration table`}>
                    {fmt(row.values[param.key])}
                    <small>{param.unit === "count" ? "×" : param.unit === "deg" ? "°" : param.unit}</small>
                    <em>{row.name}</em>
                  </span>
                ) : (
                <ScrubField
                  label={param.label}
                  chip={param.label.slice(0, 2).toUpperCase()}
                  value={def.params?.[param.key] ?? param.value}
                  formula={def.formulas?.[param.key]}
                  onFormula={(f) => run("shape.link_param", { target: def.id, key: param.key, expr: f })}
                  unit={param.unit === "count" ? "×" : param.unit === "deg" ? "°" : param.unit}
                  min={param.min ?? undefined}
                  step={param.step ?? (param.unit === "count" ? 1 : 10)}
                  onLive={() => {}}
                  onCommit={(v) => run("shape.set_params", { target: def.id, values: { [param.key]: v } })}
                />
                )}
              </div>
            ))}
          </div>
        </section>
      )}

      {prog && def && (
        <ConfigSection
          key={def.id}
          title="Component configurations"
          entity={def}
          run={run}
          onPreview={() => {}}
          users={Object.fromEntries(
            [...entities.values()].flatMap((e) => (instance(e)?.of === def.id && instance(e)?.config ? [instance(e)!.config!] : [])).reduce((m, c) => m.set(c, (m.get(c) ?? 0) + 1), new Map<string, number>()),
          )}
        />
      )}

      <div className="row-buttons">
        {uses > 1 && (
          <button type="button" className="secondary" onClick={() => run("assembly.instance.make_unique", { target: entity.id })} title="Give this instance its own copy to customise; the others stay as they are">
            <Icon name="spark" /> Make unique
          </button>
        )}
        <a className="download" href={projectUrl(`/export/${entity.id}.step?tolerance=1`)} download title="Exact B-rep of everything inside, every part named and placed">
          <Icon name="download" /> STEP · exact
        </a>
        <a className="download" href={projectUrl(`/export/${entity.id}.zip?tolerance=1`)} download>
          <Icon name="download" /> Fabrication package
        </a>
      </div>
      {stats && (
        <p className="props-hint">
          {stats.shapes.toLocaleString()} components · {stats.parts.toLocaleString()} parts · {formatMass(stats.mass_kg)} · {fmt((stats.bounds_max[2] - stats.bounds_min[2]) / 1000)} m tall
        </p>
      )}
    </div>
  );
}

// ---- Parameter sets ----------------------------------------------------------

type ParamDef = { key: string; label: string; value: number; unit: string; min?: number | null; max?: number | null; step?: number | null; expr?: string };
type ParameterSetData = { name: string; description: string; params: ParamDef[] };

export const parameterSet = (e: EntityView) => e.components["shape.parameter_set"] as ParameterSetData | undefined;

/** Who follows a parameter: every linked parameter whose formula reads it. */
function followers(id: string, key: string, entities: ReadonlyMap<string, EntityView>) {
  const needle = `#${id}.${key}`;
  const out: string[] = [];
  for (const e of entities.values()) {
    const list = (program(e)?.params ?? parameterSet(e)?.params ?? []) as ParamDef[];
    const owner = program(e)?.name ?? parameterSet(e)?.name;
    for (const p of list) if (p.expr?.includes(needle)) out.push(`${owner} · ${p.label}`);
  }
  return out;
}

/** Project-wide values that shapes link to: change one and everything following it updates. */
export function ParameterSets(props: { entities: ReadonlyMap<string, EntityView>; run: Run }) {
  // Joints hold their values as parameter sets too; they show with their assembly.
  const sets = [...props.entities.values()].filter((e) => parameterSet(e) && !e.components["assembly.joint"]);
  if (!sets.length) return null;
  return (
    <div className="param-sets">
      {sets.map((e) => {
        const set = parameterSet(e)!;
        return (
          <section key={e.id} className="param-set">
            <header>
              <Icon name="sliders" />
              <strong>{set.name}</strong>
            </header>
            {set.description && <p className="shape-description">{set.description}</p>}
            <div className="param-list">
              {set.params.map((p) => {
                const users = followers(e.id, p.key, props.entities);
                return (
                  <div key={p.key} className={`param-row ${e.formulas?.[p.key] ? "linked" : ""}`}>
                    <span className="param-label">
                      {p.label}
                      {users.length > 0 && (
                        <small className="followers" title={users.join("\n")}>
                          drives {users.length}
                        </small>
                      )}
                      {e.formulas?.[p.key] && <LinkNote formula={e.formulas[p.key]} onUnlink={() => props.run("shape.link_param", { target: e.id, key: p.key, expr: null })} />}
                    </span>
                    <ScrubField
                      label={p.label}
                      chip={p.label.slice(0, 2).toUpperCase()}
                      value={e.params?.[p.key] ?? p.value}
                      formula={e.formulas?.[p.key]}
                      onFormula={(f) => props.run("shape.link_param", { target: e.id, key: p.key, expr: f })}
                      unit={p.unit === "count" ? "×" : p.unit === "deg" ? "°" : p.unit}
                      min={p.min ?? undefined}
                      step={p.step ?? 10}
                      onLive={() => {}}
                      onCommit={(v) => props.run("parameters.set", { target: e.id, values: { [p.key]: v } })}
                    />
                  </div>
                );
              })}
            </div>
          </section>
        );
      })}
    </div>
  );
}
