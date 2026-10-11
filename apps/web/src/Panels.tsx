// Movable tool panels. One opens at a time; closing it only closes the view.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { useDraggable } from "./drag";
import { AXIS_COLORS } from "./gizmo";
import { type EntityView, type HistoryEntry, type PartStats, type ProjectInfo, type ShapeStats, projectName, projectTitle, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { AnimatedNumber } from "./AnimatedNumber";
import { type DrawingSetup, DrawingView } from "./DrawingView";
import { GearCheck } from "./GearCheck";
import { ProjectsHome } from "./ProjectsHome";
import { StressStudy } from "./StressStudy";
import { MachineConnection } from "./MachineConnection";
import {
  KNOWN_MATERIALS,
  PROCESS_NAMES,
  assemblyParts,
  fmt,
  fmtLength,
  isSheetProcess,
  partMaterial,
  stockSize,
  type Frac,
  type Shape,
  describeOperation,
  formatMass,
  material,
  materialLook,
  materialRef,
  partName,
  pivotName,
  program,
  pivotPoint,
} from "./model";
import { ScrubField } from "./ScrubField";
import type { PanelKind } from "./ui";
import type { MotionGrab, ProgramFace, StressView, TurnBody, ViewName } from "./Viewport";

export type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

const TITLES: Record<PanelKind, string> = {
  project: "Your workspace",
  objects: "Project objects",
  properties: "Properties",
  requirements: "Requirements",
  history: "Change history",
  views: "View",
  comments: "Comments",
};

export function ToolPanel(props: { kind: PanelKind; title?: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLElement>(null);
  const handle = useDraggable(ref, null);
  const title = props.title ?? TITLES[props.kind];
  return (
    <aside ref={ref} className="tool-panel floating" aria-label={title}>
      <header className="panel-handle" tabIndex={0} aria-label={`Move ${title} panel using arrow keys`} {...handle}>
        <strong>{title}</strong>
        <IconButton icon="close" label={`Close ${title}`} onClick={props.onClose} />
      </header>
      <div className="panel-body">{props.children}</div>
    </aside>
  );
}

/** Every project in the engine's folder; each opens in its own scene. */
function ProjectSwitcher() {
  const [list, setList] = useState<ProjectInfo[] | null>(null);
  const [draft, setDraft] = useState("");
  const [home, setHome] = useState(false);
  useEffect(() => {
    fetch("/projects")
      .then((r) => r.json())
      .then(setList)
      .catch(() => setList([]));
  }, []);
  const open = (name: string, isDefault: boolean) => {
    const url = new URL(location.href);
    if (isDefault) url.searchParams.delete("project");
    else url.searchParams.set("project", name);
    location.assign(url.toString());
  };
  const slug = draft.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);
  const current = (p: ProjectInfo) => (projectName ? p.name === projectName : p.default);
  return (
    <div className="project-switcher">
      <h3 className="panel-section">Projects</h3>
      <button type="button" className="secondary all-projects" onClick={() => setHome(true)}>
        <Icon name="grid" /> All projects
      </button>
      {home && <ProjectsHome onClose={() => setHome(false)} />}
      {list === null && <p className="small muted">Loading…</p>}
      {list?.map((p) => (
        <button type="button" key={p.name} className={`list-row ${current(p) ? "active" : ""}`} onClick={() => !current(p) && open(p.name, p.default)}>
          <Icon name="layers" />
          <span>
            {projectTitle(p.default ? "" : p.name)}
            <small className="row-count">{p.revisions} revisions</small>
          </span>
          {current(p) ? <Icon name="check" /> : <Icon name="chevron" />}
        </button>
      ))}
      <form
        className="new-project"
        onSubmit={(e) => {
          e.preventDefault();
          if (slug) open(slug, false);
        }}
      >
        <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="New project name" aria-label="New project name" />
        <button type="submit" className="secondary" disabled={!slug}>
          <Icon name="plus" /> Create
        </button>
      </form>
    </div>
  );
}

export function ProjectBody(props: {
  onStorage: () => void;
  onHistory: () => void;
  onRestore: () => void;
  onFocus: () => void;
}) {
  return (
    <>
      <div className="switch-app">
        <span className="app-row active">
          <Icon name="cube" /> Forma · Design
        </span>
      </div>
      <ProjectSwitcher />
      <MachineConnection />
      <button type="button" className="list-row" onClick={props.onStorage}>
        <Icon name="cloud" />
        <span>Project storage</span>
      </button>
      <button type="button" className="list-row" onClick={props.onHistory}>
        <Icon name="history" />
        <span>Change history</span>
      </button>
      <button type="button" className="list-row" onClick={props.onRestore}>
        <Icon name="eye" />
        <span>Restore all controls</span>
      </button>
      <button type="button" className="list-row" onClick={props.onFocus}>
        <Icon name="expand" />
        <span>Hide entire interface</span>
        <kbd>F</kbd>
      </button>
      <p className="small muted">
        A · Open AI &nbsp; F · Focus &nbsp; Esc · Restore
        <br />⌘ / Ctrl Z · Undo &nbsp; ⇧⌘Z · Redo &nbsp; Delete · Remove part
        <br />
        <br />
        Local Rust engine: every change is saved to the project file before it is confirmed. AI model and peers on other
        devices are not connected yet.
      </p>
    </>
  );
}

export function ObjectsBody(props: {
  parts: EntityView[];
  materials: EntityView[];
  selected: string | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  run: Run;
  /** The component tree (and stacking plan), shown in place of the flat parts list. */
  tree?: ReactNode;
  /** Project parameter sets, shown above the parts. */
  parameters?: ReactNode;
  /** Sketches in the project (click to edit). */
  sketches?: ReactNode;
  /** Bring in a STEP file as a part. */
  onImport?: (file: File) => void;
}) {
  const { run } = props;
  const used = (id: string) => props.parts.filter((p) => materialRef(p) === id).length;
  const presets = Object.values(KNOWN_MATERIALS).filter(
    (m, i, all) => all.findIndex((n) => n.name === m.name) === i && !props.materials.some((x) => material(x)!.name === m.name),
  );
  return (
    <>
      {props.parameters && (
        <>
          <h3 className="panel-section">Parameters</h3>
          {props.parameters}
        </>
      )}
      {props.sketches && (
        <>
          <h3 className="panel-section">Sketches</h3>
          {props.sketches}
        </>
      )}
      <h3 className="panel-section">Parts</h3>
      {props.parts.length === 0 && <p className="muted">No parts yet.</p>}
      {props.tree}
      {props.parts.length > 0 && (
        <a className="download" href={projectUrl("/export/project.zip")} download>
          <Icon name="download" /> Everything, for fabrication
        </a>
      )}
      {props.onImport && (
        <label className="secondary add-part import-step" title="A supplier's part or another CAD tool's model: exact geometry, named bodies">
          <Icon name="download" /> Import STEP
          <input
            type="file"
            accept=".step,.stp,.STEP,.STP"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) props.onImport!(f);
              e.target.value = "";
            }}
          />
        </label>
      )}
      <button type="button" className="secondary add-part" onClick={props.onAdd}>
        <Icon name="plus" /> Add box part
      </button>

      <h3 className="panel-section">Materials</h3>
      <p className="small muted">Changing a density updates every part that uses it.</p>
      {props.materials.map((m) => {
        const mat = material(m)!;
        return (
          <div key={m.id} className="material-row">
            <div>
              <strong>{mat.name}</strong>
              <small>
                {used(m.id)} {used(m.id) === 1 ? "part" : "parts"}
              </small>
            </div>
            <ScrubField
              label={`${mat.name} density`}
              chip="ρ"
              tone={materialLook(mat.name).color}
              unit="kg/m³"
              step={10}
              min={1}
              value={mat.density.value}
              onLive={() => undefined}
              onCommit={(v) => run("solid.material.set_density", { target: m.id, density: { value: v, unit: "kg/m3" } })}
            />
            <IconButton icon="trash" label={`Delete ${mat.name}`} onClick={() => run("core.entity.delete", { target: m.id })} />
          </div>
        );
      })}
      {presets.length > 0 && (
        <div className="presets">
          {presets.map((p) => (
            <button
              type="button"
              key={p.name}
              className="chip"
              onClick={() => run("solid.material.create", { name: p.name, density: { value: p.density, unit: "kg/m3" } })}
            >
              <Icon name="plus" /> {p.name}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

const SIZE_TONE = "#8b6fd8";

/** Two-letter chip from a parameter label: "Head height" → "HH", "Pitch" → "P". */
function paramChip(label: string) {
  const words = label.split(/\s+/).filter(Boolean);
  return (words.length > 1 ? words.slice(0, 2).map((w) => w[0]) : [label.slice(0, 1)]).join("").toUpperCase();
}
const AXIS_TONES = { x: AXIS_COLORS[0], y: AXIS_COLORS[1], z: AXIS_COLORS[2] };

export type PropertiesProps = {
  part?: EntityView;
  /** Shape currently shown (live edit or engine value), millimeters. */
  shape: Shape | null;
  previewing: boolean;
  materials: EntityView[];
  lockRatio: boolean;
  onToggleLock: () => void;
  pivot: Frac;
  pivotMode: boolean;
  onTogglePivot: () => void;
  /** Live pivot edit (axis 0–2, world mm) or null to cancel; commit sets it. */
  onPivotLive: (axis: 0 | 1 | 2, value: number | null) => void;
  onPivotCommit: (axis: 0 | 1 | 2, value: number) => void;
  /** AI-made shapes: measured stats (live while a parameter is previewed). */
  shapeStats: ShapeStats | null;
  shapePreviewing: boolean;
  onParamLive: (key: string, value: number | null) => void;
  onParamCommit: (key: string, value: number) => void;
  /** Link a parameter to a formula ("=Desk size.width"), or unlink it (null). */
  onParamFormula: (key: string, formula: string | null) => void;
  /** Start picking edges to round or bevel; remove one of the part's edge finishes. */
  onEdges: () => void;
  onRemoveFinish: (index: number) => void;
  onLive: (patch: Partial<Shape> | null) => void;
  onCommit: (patch: Partial<Shape>) => void;
  onAssign: (materialId: string | null) => void;
  onCreateMaterial: (name: string, density: number) => void;
  onDelete: () => void;
  /** Every entity, to resolve what each assembly part is made of. */
  entities: ReadonlyMap<string, EntityView>;
  /** Commits a command (as the person). */
  run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
  /** Picks a face in the view for a stress study (purpose is shown as a hint). */
  onPickFace: (purpose: string, take: (f: ProgramFace) => void) => void;
  /** Lets the view drive a mechanism by hand (null stops it). */
  onGrab: (spec: MotionGrab | null) => void;
  /** Shows a stress study in the view (null clears it). */
  onShowStress: (id: string, view: StressView | null) => void;
  /** Shows a mechanism turned in the view (null puts it back). */
  onTurn: (id: string, bodies: TurnBody[] | null, options?: { ghost?: boolean }) => void;
  /** Picked part of the selected assembly, by name. */
  selectedPart: string | null;
  onSelectPart: (name: string | null) => void;
};

function StatusPill({ status }: { status: EntityView["status"] }) {
  if (!status) return null;
  const text = { current: "Up to date", stale: "Stale", computing: "Computing", failed: "Failed", blocked: "Blocked" }[status.state];
  return (
    <span className={`status-pill ${status.state}`} title={status.reason ?? text}>
      <Icon name={status.state === "current" ? "check" : "alert"} />
      {text}
    </span>
  );
}

function Swatch(props: { name: string; detail?: string; color?: string; active: boolean; onClick: () => void; dashed?: boolean }) {
  return (
    <button type="button" className={`swatch ${props.active ? "active" : ""}`} onClick={props.onClick} aria-pressed={props.active} title={props.detail ? `${props.name} · ${props.detail}` : props.name}>
      <span className={`swatch-ball ${props.dashed ? "none" : ""}`} style={props.color ? ({ "--ball": props.color } as React.CSSProperties) : undefined} />
      <span className="swatch-name">{props.name}</span>
    </button>
  );
}

/** Under a linked parameter: what drives it, and a way to cut the link. */
export function LinkNote({ formula, onUnlink }: { formula: string; onUnlink: () => void }) {
  return (
    <span className="link-note">
      <Icon name="link" />
      <code title={`Follows ${formula}`}>{formula.replace(/^=/, "")}</code>
      <button type="button" onClick={onUnlink} title="Keep the current value and stop following">
        Unlink
      </button>
    </span>
  );
}

export function PropertiesBody(props: PropertiesProps) {
  const { part, shape } = props;
  if (!part || !shape) {
    return (
      <div className="props-empty">
        <div className="props-empty-art" aria-hidden>
          <Icon name="cursor" />
        </div>
        <p>Select a part to shape it.</p>
        <small>Click a part in the view, then drag its purple arrows to resize or drag the part to move it.</small>
      </div>
    );
  }
  const ref = materialRef(part);
  const current = props.materials.find((m) => m.id === ref);
  const density = current ? material(current)!.density.value : null;
  const prog = program(part);
  const volume = prog ? (props.shapeStats?.volume_mm3 ?? 0) / 1e9 : (shape.width * shape.depth * shape.height) / 1e9; // m³
  // Shapes (and assemblies with a material per part) are weighed by the engine.
  const mass = prog ? (props.shapePreviewing ? (density === null ? null : volume * density) : (part.mass?.mass_kg ?? null)) : density === null ? null : volume * density;
  const presets = Object.values(KNOWN_MATERIALS).filter(
    (m, i, all) => all.findIndex((n) => n.name === m.name) === i && !props.materials.some((x) => material(x)!.name === m.name),
  );
  // Assemblies whose parts name their own materials: summarise instead of one picker.
  const asmParts = assemblyParts(part);
  const perPart = (() => {
    if (!asmParts.length || asmParts.some((p) => !p.material)) return [];
    const byName = new Map<string, { name: string; count: number; mass: number | null }>();
    asmParts.forEach((p, i) => {
      const mat = partMaterial(part, p.material, props.entities);
      const name = (mat && material(mat)?.name) ?? p.material!;
      const row = byName.get(name) ?? { name, count: 0, mass: 0 };
      row.count++;
      const m = part.part_masses?.[i];
      row.mass = row.mass === null || m == null ? null : row.mass + m;
      byName.set(name, row);
    });
    return [...byName.values()].sort((a, b) => (b.mass ?? 0) - (a.mass ?? 0));
  })();
  const at = pivotPoint(shape, props.pivot);
  const shown: Shape = { ...shape, x: at[0], y: at[1], z: at[2] };
  const field = (key: keyof Shape, label: string, chip: string, tone: string, min?: number) => (
    <ScrubField
      key={key}
      label={label}
      chip={chip}
      tone={tone}
      value={shown[key]}
      min={min}
      onLive={(v) => props.onLive(v === null ? null : { [key]: v })}
      onCommit={(v) => props.onCommit({ [key]: v })}
    />
  );

  return (
    <div className="props">
      <div className="props-head">
        <span className="part-glyph" style={{ "--ball": materialLook(current ? material(current)!.name : undefined).color } as React.CSSProperties} aria-hidden />
        <div className="props-title">
          <strong>{partName(part)}</strong>
          <small>
            {prog
              ? `${prog.source.includes("importStep(") ? "Imported STEP · exact geometry" : "AI-made shape"} · ${(props.shapeStats?.triangles ?? 0).toLocaleString()} triangles`
              : `Box part · ${part.id.slice(-6)}`}
          </small>
        </div>
        <StatusPill status={part.status} />
      </div>

      {prog ? (
        <section className="props-section">
          <header>
            <h3>Parameters</h3>
            {props.shapePreviewing && <span className="section-note live-note">Previewing…</span>}
          </header>
          {prog.description && <p className="shape-description">{prog.description}</p>}
          <div className="param-list">
            {prog.params.map((param) => (
              <div key={param.key} className={`param-row ${part.formulas?.[param.key] ? "linked" : ""}`}>
                <span className="param-label">
                  {param.label}
                  {part.formulas?.[param.key] && <LinkNote formula={part.formulas[param.key]} onUnlink={() => props.onParamFormula(param.key, null)} />}
                </span>
                <ScrubField
                  label={param.label}
                  chip={paramChip(param.label)}
                  tone={SIZE_TONE}
                  value={part.params?.[param.key] ?? param.value}
                  formula={part.formulas?.[param.key]}
                  onFormula={(f) => props.onParamFormula(param.key, f)}
                  unit={param.unit === "count" ? "×" : param.unit === "deg" ? "°" : param.unit}
                  min={param.min ?? undefined}
                  step={param.step ?? (param.unit === "count" ? 1 : param.unit === "deg" ? 1 : 0.5)}
                  onLive={(v) => props.onParamLive(param.key, v)}
                  onCommit={(v) => props.onParamCommit(param.key, v)}
                />
              </div>
            ))}
          </div>
          <div className="finish-list">
            {(prog.finishes ?? []).map((f, i) => {
              const size = prog.params.find((q) => q.key === f.key);
              return (
                <div key={i} className="finish-row">
                  <span className="finish-kind">{f.kind === "fillet" ? "Fillet" : f.kind === "chamfer" ? "Chamfer" : "Push"}</span>
                  <span className="finish-what">
                    {f.edges.length} {f.kind === "push" ? (f.edges.length === 1 ? "face" : "faces") : f.edges.length === 1 ? "edge" : "edges"}
                    {f.part ? ` of ${f.part}` : ""} · {fmt(part.params?.[f.key] ?? size?.value ?? 0)} mm
                    {part.formulas?.[f.key] ? " · linked" : ""}
                  </span>
                  <button type="button" className="ib" title="Remove this finish" aria-label="Remove this finish" onClick={() => props.onRemoveFinish(i)}>
                    <Icon name="close" />
                  </button>
                </div>
              );
            })}
            <button type="button" className="secondary finish-add" onClick={props.onEdges}>
              <Icon name="fillet" /> Round or bevel edges <kbd>E</kbd>
            </button>
          </div>
        </section>
      ) : (
      <section className="props-section">
          <header>
            <h3>Size</h3>
            <button
              type="button"
              className={`lock ${props.lockRatio ? "active" : ""}`}
              onClick={props.onToggleLock}
              aria-pressed={props.lockRatio}
              title={props.lockRatio ? "Proportions locked" : "Lock proportions"}
            >
              <Icon name="link" />
              {props.lockRatio ? "Locked" : "Lock"}
            </button>
          </header>
          <div className="scrub-grid">
            {field("width", "Width", "W", SIZE_TONE, 0.1)}
            {field("depth", "Depth", "D", SIZE_TONE, 0.1)}
            {field("height", "Height", "H", SIZE_TONE, 0.1)}
          </div>
        </section>
      )}

      {props.pivotMode && (
        <section className="props-section pivot-section">
          <header>
            <h3>Pivot</h3>
            <span className="section-note">{pivotName(props.pivot)}</span>
            <button type="button" className="lock active" onClick={props.onTogglePivot}>
              <Icon name="check" /> Done
            </button>
          </header>
          <div className="scrub-grid">
            {([0, 1, 2] as const).map((k) => (
              <ScrubField
                key={k}
                label={`Pivot ${"XYZ"[k]}`}
                chip={"XYZ"[k]}
                tone={AXIS_TONES[(["x", "y", "z"] as const)[k]]}
                value={at[k]}
                onLive={(v) => props.onPivotLive(k, v)}
                onCommit={(v) => props.onPivotCommit(k, v)}
              />
            ))}
          </div>
          <p className="props-hint">
            <Icon name="spark" /> Only the pivot moves. Drag its gizmo in the view, click a point or the surface, or type exact values.
          </p>
        </section>
      )}

      <section className="props-section">
        <header>
          <h3>Position</h3>
          <button
            type="button"
            className={`lock pivot-button ${props.pivotMode ? "active" : ""}`}
            onClick={props.onTogglePivot}
            aria-pressed={props.pivotMode}
            title="Choose where the part moves and rotates from (P)"
          >
            <span className="pivot-glyph" aria-hidden />
            {props.pivotMode ? "Pick a point…" : pivotName(props.pivot)}
          </button>
        </header>
        <div className="scrub-grid">
          {field("x", "X position", "X", AXIS_TONES.x)}
          {field("y", "Y position", "Y", AXIS_TONES.y)}
          {field("z", "Z position", "Z", AXIS_TONES.z)}
        </div>
      </section>

      {perPart.length > 0 ? (
        <section className="props-section">
          <header>
            <h3>Materials</h3>
            <span className="section-note">Set per part by the program</span>
          </header>
          <div className="material-mix">
            {perPart.map((m) => (
              <div key={m.name} className="material-mix-row">
                <span className="row-ball" style={{ "--ball": materialLook(m.name).color } as React.CSSProperties} aria-hidden />
                <strong>{m.name}</strong>
                <small>
                  {m.count} {m.count === 1 ? "part" : "parts"}
                  {m.mass != null && <> · {formatMass(m.mass)}</>}
                </small>
              </div>
            ))}
          </div>
        </section>
      ) : (
      <section className="props-section">
        <header>
          <h3>Material</h3>
          {current && <span className="section-note">{material(current)!.density.value.toLocaleString()} kg/m³</span>}
        </header>
        <div className="swatches">
          <Swatch name="None" dashed active={!ref} onClick={() => props.onAssign(null)} />
          {props.materials.map((m) => (
            <Swatch
              key={m.id}
              name={material(m)!.name}
              detail={`${material(m)!.density.value.toLocaleString()} kg/m³`}
              color={materialLook(material(m)!.name).color}
              active={m.id === ref}
              onClick={() => props.onAssign(m.id)}
            />
          ))}
          {presets.slice(0, 4).map((p) => (
            <button type="button" key={p.name} className="swatch add" onClick={() => props.onCreateMaterial(p.name, p.density)} title={`Add ${p.name} and use it`}>
              <span className="swatch-ball ghost" style={{ "--ball": materialLook(p.name).color } as React.CSSProperties}>
                <Icon name="plus" />
              </span>
              <span className="swatch-name">{p.name}</span>
            </button>
          ))}
        </div>
      </section>
      )}

      <div className={`stats ${props.previewing ? "is-preview" : ""}`}>
        <div>
          <span>Mass{props.previewing && <i>preview</i>}</span>
          <strong>{mass === null ? "No material" : <AnimatedNumber value={mass} format={(v) => formatMass(v)} />}</strong>
        </div>
        <div>
          <span>Volume</span>
          <strong>
            <AnimatedNumber value={volume * 1e6} format={(v) => `${v.toFixed(2)} cm³`} />
          </strong>
        </div>
      </div>

      {prog && part.shape?.moves && <GearCheck id={part.id} version={part.mesh} motion={part.shape.motion} linkage={!!part.shape.linkage} parts={part.shape.parts ?? []} onTurn={props.onTurn} onShowStress={props.onShowStress} onGrab={props.onGrab} />}

      {prog && (!part.shape?.parts?.length || props.selectedPart) && (
        <StressStudy
          id={part.id}
          part={props.selectedPart}
          version={part.mesh}
          material={(props.selectedPart ? part.shape?.parts?.find((p) => p.name === props.selectedPart)?.material : null) ?? (current ? material(current)?.name : null) ?? null}
          bounds={(() => {
            const b = props.selectedPart ? part.shape?.parts?.find((p) => p.name === props.selectedPart) : part.shape;
            return b ? { min: b.bounds_min, max: b.bounds_max } : null;
          })()}
          saved={part.components["shape.studies"]}
          run={props.run}
          onPickFace={props.onPickFace}
          onShow={props.onShowStress}
        />
      )}

      {prog && props.shapeStats && (
        <div className="shape-facts">
          <span>
            <Icon name="check" /> Watertight solid
          </span>
          <span>{props.shapeStats.genus === 0 ? "No through-holes" : `${props.shapeStats.genus} through-hole${props.shapeStats.genus === 1 ? "" : "s"}`}</span>
          <span>{(props.shapeStats.area_mm2 / 100).toLocaleString("en-US", { maximumFractionDigits: 1 })} cm² surface</span>
          <span title="The viewport shows the formula with curves within this distance of the true shape. Checks and exports use their own, finer accuracy.">
            Shown at ±{Number((prog.display_tolerance ?? 0.01).toPrecision(2))} mm
          </span>
        </div>
      )}

      <MakeSections
        part={part}
        parts={assemblyParts(part)}
        entities={props.entities}
        exact={!prog}
        displayTolerance={prog?.display_tolerance ?? 0.01}
        selectedPart={props.selectedPart}
        onSelectPart={props.onSelectPart}
        run={props.run}
      />

      {prog && (
        <details className="source">
          <summary>
            <Icon name="spark" /> Program · {prog.source.split("\n").length} lines · written by Forma AI
          </summary>
          <pre>
            <code>{prog.source}</code>
          </pre>
        </details>
      )}

      <p className="props-hint">
        <Icon name="spark" />{" "}
        {prog ? (
          <>Drag a parameter to reshape it live. Ask Forma AI for anything the parameters can’t do; it rewrites the program.</>
        ) : (
          <>
            Drag the arrows on the part, drag a field label to scrub, or type math like <kbd>+10</kbd> <kbd>2in</kbd>
          </>
        )}
      </p>

      <button type="button" className="delete-part" onClick={props.onDelete}>
        <Icon name="trash" /> Delete part
      </button>
    </div>
  );
}

/** Export accuracies relative to how finely the shape is shown: a bolt at ±0.01 mm, a tower at ±5 mm. */
function accuracies(shown: number) {
  const round = (v: number) => Number(v.toPrecision(2));
  return [
    { label: "Draft", value: round(shown * 5), hint: "Fast preview" },
    { label: "Standard", value: round(shown), hint: "As shown in the view" },
    { label: "Fine", value: round(shown / 2), hint: "Precision work" },
    { label: "Ultra", value: round(shown / 10), hint: "Inspection, fits" },
  ];
}

/**
 * Making it: every part of an assembly as its own item (process, stock size,
 * mass, its own files), then downloads at a chosen accuracy. The engine
 * re-derives the formula for each export.
 */
function MakeSections(props: {
  part: EntityView;
  parts: PartStats[];
  entities: ReadonlyMap<string, EntityView>;
  exact: boolean;
  displayTolerance: number;
  selectedPart: string | null;
  onSelectPart: (name: string | null) => void;
  run: Run;
}) {
  const { part, parts } = props;
  const presets = accuracies(props.displayTolerance);
  const [accuracy, setAccuracy] = useState(presets[1].value);
  const picked = useRef<HTMLButtonElement>(null);
  const [drawing, setDrawing] = useState(false);
  // A block body: smooth scrollIntoView returns a promise, which React would call as a cleanup.
  useEffect(() => {
    picked.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [props.selectedPart]);
  const masses = part.part_masses ?? [];
  const sheet = parts.filter((p) => isSheetProcess(p.process)).length;
  const q = (name: string) => `part=${encodeURIComponent(name)}&tolerance=${accuracy}`;
  return (
    <>
      {parts.length > 0 && (
        <section className="props-section parts-section">
          <header>
            <h3>Parts</h3>
            <span className="section-note">
              {parts.length} {parts.length === 1 ? "item" : "items"} to make · click one in the view
            </span>
          </header>
          <div className="part-list">
            {parts.map((p, i) => {
              const mat = partMaterial(part, p.material, props.entities);
              const matName = mat ? material(mat)?.name : undefined;
              const [l, w, t] = stockSize(p);
              const active = props.selectedPart === p.name;
              return (
                <div key={p.name} className={`part-row ${active ? "active" : ""}`}>
                  <button type="button" ref={active ? picked : undefined} className="part-pick" onClick={() => props.onSelectPart(active ? null : p.name)}>
                    <span className="row-ball" style={{ "--ball": materialLook(matName).color } as React.CSSProperties} aria-hidden />
                    <span className="part-text">
                      <strong>{p.name}</strong>
                      <small>
                        {[matName ?? "No material", p.process ? PROCESS_NAMES[p.process] : null].filter(Boolean).join(" · ")}
                      </small>
                      <small className="part-size">
                        {fmtLength(l)} × {fmtLength(w)} × {fmtLength(t)}
                        {masses[i] != null && <> · {formatMass(masses[i])}</>}
                      </small>
                    </span>
                  </button>
                  <span className="part-files">
                    <a href={projectUrl(`/export/${part.id}.step?${q(p.name)}`)} download title={`${p.name} as exact STEP (B-rep), at the origin`}>
                      STEP
                    </a>
                    <a href={projectUrl(`/export/${part.id}.stl?${q(p.name)}`)} download title={`${p.name} as STL, at the origin`}>
                      STL
                    </a>
                    {isSheetProcess(p.process) && (
                      <a href={projectUrl(`/export/${part.id}.dxf?${q(p.name)}`)} download title={`${p.name} laid flat as a layered DXF: one layer per cut depth`}>
                        DXF
                      </a>
                    )}
                  </span>
                </div>
              );
            })}
          </div>
        </section>
      )}

      <section className="props-section export-section">
        <header>
          <h3>Export</h3>
          <span className="section-note">{props.exact ? "Box faces are exact" : "Curves within the chosen accuracy"}</span>
        </header>
        <div className="accuracy" role="radiogroup" aria-label="Export accuracy">
          {presets.map((a) => (
            <button
              type="button"
              key={a.label}
              role="radio"
              aria-checked={accuracy === a.value}
              className={accuracy === a.value ? "active" : ""}
              onClick={() => setAccuracy(a.value)}
              title={a.hint}
            >
              <strong>{a.label}</strong>
              <span>±{a.value} mm</span>
            </button>
          ))}
        </div>
        {parts.length > 0 && (
          <a className="download primary" href={projectUrl(`/export/${part.id}.zip?tolerance=${accuracy}`)} download>
            <Icon name="download" /> Fabrication package
            <small>
              STEP + STL per part{sheet > 0 ? ` · ${sheet} DXF cut files` : ""} · cut list
            </small>
          </a>
        )}
        <div className="download-pair">
          <a className="download" href={projectUrl(`/export/${part.id}.step?tolerance=${accuracy}`)} download title="Exact B-rep: true planes, cylinders and curves, every part named. For machining, other CAD tools and suppliers.">
            <Icon name="download" /> STEP · exact
          </a>
          <a className="download" href={projectUrl(`/export/${part.id}.stl?tolerance=${accuracy}`)} download title="Triangle mesh within the chosen accuracy. For 3D printing.">
            <Icon name="download" /> {parts.length ? "STL · whole" : "STL"}
          </a>
        </div>
        <button type="button" className="download drawing-open" onClick={() => setDrawing(true)} title="Front, top, side and isometric views from the exact solid, dimensioned, with a title block. PDF or DXF.">
          <Icon name="ruler" /> Engineering drawing
          <small>Views, dimensions, title block · PDF or DXF</small>
        </button>
        {drawing && (
          <DrawingView
            id={part.id}
            name={partName(part)}
            parts={parts.map((p) => p.name)}
            setups={(part.components["shape.drawings"] as DrawingSetup[] | undefined) ?? []}
            onSave={(setup) => props.run("shape.set_drawing", { target: part.id, drawing: setup })}
            onClose={() => setDrawing(false)}
          />
        )}
        <PrintRow id={part.id} />
      </section>
    </>
  );
}

/** Straight to the slicer: the part as one 3MF object, opened in Bambu Studio on this computer. */
function PrintRow({ id }: { id: string }) {
  const [state, setState] = useState<{ kind: "idle" | "busy" | "opened" | "saved" | "error"; text?: string }>({ kind: "idle" });
  useEffect(() => setState({ kind: "idle" }), [id]);
  const send = async () => {
    setState({ kind: "busy" });
    try {
      const r = await fetch(projectUrl(`/slicer/${id}`), { method: "POST" });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { opened: boolean; path: string };
      const where = out.path.replace(/^\/Users\/[^/]+/, "~");
      setState(out.opened ? { kind: "opened", text: where } : { kind: "saved", text: where });
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };
  return (
    <div className="print-row">
      <div className="download-pair">
        <button type="button" className="download primary" onClick={send} disabled={state.kind === "busy"} title="One 3MF object (separate bodies stay separate, so print-in-place parts print whole), opened in Bambu Studio">
          <Icon name="printer" /> {state.kind === "busy" ? "Sending…" : "Send to Bambu Studio"}
        </button>
        <a className="download" href={projectUrl(`/export/${id}.3mf?tolerance=0.005`)} download title="3MF: one object, millimetres, for any slicer">
          <Icon name="download" /> 3MF
        </a>
      </div>
      {state.kind === "opened" && <p className="print-note ok">Opened in Bambu Studio · saved as {state.text}</p>}
      {state.kind === "saved" && (
        <p className="print-note">
          Saved as {state.text}. Bambu Studio isn't installed on this computer: get it at <b>bambulab.com/download</b>, then send again.
        </p>
      )}
      {state.kind === "error" && <p className="print-note bad">{state.text}</p>}
    </div>
  );
}

function timeAgo(at?: number) {
  if (!at) return "Earlier";
  const s = Math.round((Date.now() - at) / 1000);
  if (s < 60) return "Just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function HistoryBody(props: { history: HistoryEntry[]; userId: string; onUndo: () => void; onRedo: () => void }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), 30000);
    return () => window.clearInterval(t);
  }, []);
  if (props.history.length === 0) {
    return <p className="muted">Your edits will appear here automatically. Every change is saved as a revision you can undo.</p>;
  }
  return (
    <>
      {props.history.slice(0, 60).map((h) => (
        <div key={h.revision} className="history-row">
          <Icon name="history" />
          <span>
            {describeOperation(h.operation)}
            <small>
              Revision {h.revision} · {h.user_id === props.userId ? "You" : h.user_id} · {timeAgo(h.at)}
            </small>
          </span>
        </div>
      ))}
      <div className="row-buttons">
        <button type="button" className="secondary" onClick={props.onUndo}>
          <Icon name="undo" /> Undo my latest
        </button>
        <button type="button" className="secondary" onClick={props.onRedo}>
          <Icon name="redo" /> Redo
        </button>
      </div>
    </>
  );
}

export function ViewsBody(props: {
  view: ViewName;
  wireframe: boolean;
  turntable: boolean;
  onView: (v: ViewName) => void;
  onFit: () => void;
  onWireframe: () => void;
  onTurntable: () => void;
}) {
  return (
    <>
      {(["perspective", "front", "top", "bottom"] as ViewName[]).map((v) => (
        <button type="button" key={v} className={`list-row ${props.view === v ? "active" : ""}`} onClick={() => props.onView(v)}>
          <Icon name="cube" />
          <span>{v[0].toUpperCase() + v.slice(1)}</span>
        </button>
      ))}
      <button type="button" className="list-row" onClick={props.onFit}>
        <Icon name="fit" />
        <span>Fit all parts</span>
      </button>
      <button type="button" className="list-row" onClick={props.onWireframe}>
        <Icon name="grid" />
        <span>{props.wireframe ? "Solid shading" : "Wireframe"}</span>
      </button>
      <button type="button" className={`list-row ${props.turntable ? "active" : ""}`} onClick={props.onTurntable}>
        <Icon name="rotate" />
        <span>{props.turntable ? "Stop turntable" : "Turntable"}</span>
      </button>
    </>
  );
}
