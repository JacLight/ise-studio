// Scans and inspection: point clouds and meshes from scanners or
// photogrammetry, added to the project and shown in place; a mesh's check
// (holes, non-manifold edges, flipped, degenerate and crossing triangles)
// with markers where they are, repaired into a new scan; reworked
// (decimate, smooth, remesh; thin, clean, reconstruct points); and compared
// with the CAD part it should match — a colour map of every point's
// deviation, the statistics, pass or fail, and a requirement that keeps
// checking it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { isPart, partName, program } from "./model";
import { PhotoScan } from "./PhotoScan";
import { DEVIATION_STOPS, type ScanLayer, deviationColor } from "./scanLayer";
import "./ScanTools.css";

type V3 = [number, number, number];
type Entry = { id: string; name: string; kind: "points" | "mesh"; format: string; points: number; triangles: number; bounds: number[]; source: string; parent?: string };
type Report = {
  vertices: number;
  triangles: number;
  components: number;
  holes: number;
  open_edges: number;
  non_manifold_edges: number;
  non_manifold_vertices: number;
  flipped: number;
  degenerate: number;
  duplicate: number;
  self_intersections: number;
  unused_vertices: number;
  solid: boolean;
  volume: number;
  area: number;
  at: { holes: V3[][]; non_manifold_edges: [V3, V3][]; non_manifold_vertices: V3[]; flipped: V3[]; degenerate: V3[]; self_intersections: V3[] };
};
type Stats = { count: number; min: number; max: number; mean: number; mean_abs: number; rms: number; std: number; p95: number };
type Inspection = {
  stats: Stats;
  tolerance: number;
  within: number;
  over: number;
  under: number;
  passed: boolean;
  required: number;
  histogram: number[];
  alignment: { transform: number[][]; rms_before: number; rms_after: number } | null;
  step: number;
  values: number[];
  worst: { at: V3; deviation: number }[];
  scan: Entry;
  target: { id: string; name: string; part: string | null };
};
type View = { kind: string; step: number; count: number; positions: number[]; colors: number[]; mesh?: { positions: number[]; triangles: number[] } };

const MARKS: { key: keyof Report["at"]; label: string; color: string }[] = [
  { key: "holes", label: "Holes", color: "#e0402f" },
  { key: "non_manifold_edges", label: "Non-manifold edges", color: "#b04fd6" },
  { key: "non_manifold_vertices", label: "Non-manifold vertices", color: "#b04fd6" },
  { key: "flipped", label: "Flipped triangles", color: "#f08a24" },
  { key: "degenerate", label: "Degenerate triangles", color: "#7a7f87" },
  { key: "self_intersections", label: "Self-intersections", color: "#d6455d" },
];

const r3 = (v: number) => (Math.abs(v) >= 100 ? v.toFixed(1) : Math.abs(v) >= 1 ? v.toFixed(2) : v.toFixed(3));
const signed = (v: number) => `${v >= 0 ? "+" : "−"}${r3(Math.abs(v))}`;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const r = await fetch(projectUrl(path), { method, body: body === undefined ? undefined : body instanceof ArrayBuffer ? body : JSON.stringify(body) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export function ScanTools(props: {
  entities: ReadonlyMap<string, EntityView>;
  onJob: (listener: (job: unknown) => void) => () => void;
  run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
  notify: (message: string) => void;
  onLayer: (layer: ScanLayer | null) => void;
  onClose: () => void;
  /** Place a few separate views (side, top, front) as reference pictures to model over. */
  onReferencePhotos?: () => void;
}) {
  const [scans, setScans] = useState<Entry[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [view, setView] = useState<View | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const [inspection, setInspection] = useState<Inspection | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [target, setTarget] = useState<string>("");
  const [tolerance, setTolerance] = useState(0.2);
  const [align, setAlign] = useState<"none" | "icp" | "best">("none");
  const [ratio, setRatio] = useState(0.5);
  const fileInput = useRef<HTMLInputElement>(null);

  const entry = scans.find((s) => s.id === active) ?? null;
  const parts = useMemo(() => [...props.entities.values()].filter((e) => isPart(e) && (program(e) || e.components["assembly.instance"])), [props.entities]);

  const refresh = useCallback(async (select?: string) => {
    const list = await call<Entry[]>("GET", "/scans");
    setScans(list);
    if (select) setActive(select);
  }, []);
  useEffect(() => void refresh().catch((e) => setError(String(e.message ?? e))), [refresh]);
  useEffect(() => () => props.onLayer(null), []); // eslint-disable-line react-hooks/exhaustive-deps

  // A new scan picked: fetch what to draw, forget the old check and inspection.
  useEffect(() => {
    setReport(null);
    setInspection(null);
    setView(null);
    if (!active) return;
    call<View>("GET", `/scans/${active}/view?max=250000`)
      .then(setView)
      .catch((e) => setError(String(e.message ?? e)));
  }, [active]);

  // Draw: the scan (coloured by deviation when inspected), and problem markers when checked.
  useEffect(() => {
    if (!view) return props.onLayer(null);
    let positions = new Float32Array(view.positions);
    let colors: Float32Array | null = view.colors.length ? new Float32Array(view.colors.map((c) => c / 255)) : null;
    if (inspection && inspection.values.length) {
      // Aligned points are drawn where the alignment put them.
      const t = inspection.alignment?.transform;
      if (t) {
        const moved = new Float32Array(positions.length);
        for (let i = 0; i < positions.length; i += 3) {
          const [x, y, z] = [positions[i], positions[i + 1], positions[i + 2]];
          for (let k = 0; k < 3; k++) moved[i + k] = t[k][0] * x + t[k][1] * y + t[k][2] * z + t[k][3];
        }
        positions = moved;
      }
      const n = Math.min(inspection.values.length, positions.length / 3);
      const c = new Float32Array(n * 3);
      const color = new THREE.Color();
      const range = Math.max(Math.abs(inspection.stats.min), Math.abs(inspection.stats.max), inspection.tolerance * 2);
      for (let i = 0; i < n; i++) {
        deviationColor(inspection.values[i], inspection.tolerance, range, color);
        c.set([color.r, color.g, color.b], i * 3);
      }
      positions = positions.slice(0, n * 3);
      colors = c;
    }
    const markers: ScanLayer["markers"] = [];
    const lines: ScanLayer["lines"] = [];
    if (report) {
      for (const m of MARKS) {
        if (m.key === "holes") report.at.holes.forEach((h) => lines!.push({ points: h, color: m.color, closed: true }));
        else if (m.key === "non_manifold_edges") report.at.non_manifold_edges.forEach((e) => lines!.push({ points: e, color: m.color }));
        else markers!.push({ at: report.at[m.key] as V3[], color: m.color });
      }
    }
    if (inspection) markers!.push({ at: inspection.worst.slice(0, 3).map((w) => w.at), color: "#111", size: 7 });
    props.onLayer({
      positions,
      colors,
      mesh: view.mesh && !inspection ? { positions: new Float32Array(view.mesh.positions), triangles: new Uint32Array(view.mesh.triangles) } : null,
      markers,
      lines,
      size: view.count > 100000 ? 1.5 : 2.5,
    });
  }, [view, report, inspection]); // eslint-disable-line react-hooks/exhaustive-deps

  const act = async (label: string, f: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  };

  const upload = (files: FileList | null) => {
    const f = files?.[0];
    if (!f) return;
    void act("Adding", async () => {
      const e = await call<Entry>("POST", `/scans/${encodeURIComponent(f.name)}`, await f.arrayBuffer());
      await refresh(e.id);
      setNote(`${e.name}: ${e.kind === "mesh" ? `${e.triangles.toLocaleString()} triangles` : `${e.points.toLocaleString()} points`}`);
    });
  };

  const derive = (label: string, path: string, body: unknown = {}) =>
    act(label, async () => {
      const r = await call<{ scan: Entry; report?: Record<string, unknown> }>("POST", `/scans/${active}/${path}`, body);
      await refresh(r.scan.id);
      const t = r.report as Record<string, number[] & number> | undefined;
      if (t?.after) {
        const after = t.after as unknown as Report;
        setNote(`Repaired: ${(t.steps as unknown as { step: string; count: number }[]).map((s) => `${s.count} ${s.step}`).join(", ") || "nothing to fix"}. ${after.solid ? "Now a closed solid." : "Still not a closed solid."}`);
      } else if (t?.triangles) setNote(`${label}: ${t.triangles[0].toLocaleString()} → ${t.triangles[1].toLocaleString()} triangles${t.max_deviation != null ? `, within ${r3(t.max_deviation as unknown as number)} mm of the original` : ""}`);
      else if (t?.points) setNote(`${label}: ${t.points[0].toLocaleString()} → ${t.points[1].toLocaleString()} points`);
      else if (t?.closed != null) setNote(`Surface: ${(t.triangles as unknown as number)?.toLocaleString?.() ?? ""} triangles, ${t.closed ? "closed" : "open"}, passes the points within RMS ${r3(t.rms as unknown as number)} mm`);
    });

  const check = () =>
    act("Checking", async () => {
      setReport(await call<Report>("POST", `/scans/${active}/query/check`, {}));
    });

  const fit = () =>
    act("Fitting", async () => {
      const f = await call<Record<string, unknown> & { kind: string; rms: number; points: number }>("POST", `/scans/${active}/query/fit`, { kind: "auto" });
      const params = Object.entries(f)
        .filter(([k]) => !["kind", "rms", "max", "points", "of", "alternatives"].includes(k))
        .map(([k, v]) => `${k.replace(/_/g, " ")} ${Array.isArray(v) ? `(${(v as number[]).map(r3).join(", ")})` : typeof v === "number" ? r3(v) : v}`)
        .join(" · ");
      setNote(`Best fit: ${f.kind} — ${params} (RMS ${r3(f.rms)} mm)`);
    });

  const inspect = () =>
    act("Inspecting", async () => {
      if (!target) throw new Error("Pick the part the scan should match");
      setInspection(await call<Inspection>("POST", `/scans/${active}/inspect`, { target, tolerance, align, max: 250000 }));
    });

  const requireIt = () =>
    act("Adding requirement", async () => {
      if (!inspection) return;
      await props.run("intent.requirement.create", { rule: { kind: "scan_deviation", part: inspection.target.id, scan: inspection.scan.id, tolerance: { value: tolerance, unit: "mm" }, ...(align !== "none" ? { align: true } : {}) } });
      props.notify(`Requirement added: ${inspection.target.name} matches ${inspection.scan.name} within ±${tolerance} mm`);
    });

  const makePart = () =>
    act("Making a part", async () => {
      const r = await call<{ id: string; name: string }>("POST", `/scans/${active}/shape`, entry?.kind === "mesh" ? { repair: report ? !report.solid : true } : {});
      props.notify(`Part “${r.name}” made from the scan`);
    });

  const counts: [string, number, string][] = report
    ? [
        ["Holes", report.holes, "#e0402f"],
        ["Non-manifold", report.non_manifold_edges + report.non_manifold_vertices, "#b04fd6"],
        ["Flipped", report.flipped, "#f08a24"],
        ["Degenerate", report.degenerate, "#7a7f87"],
        ["Duplicate", report.duplicate, "#7a7f87"],
        ["Crossing", report.self_intersections, "#d6455d"],
        ["Pieces", report.components, "#55606e"],
      ]
    : [];

  return (
    <aside className="tool-panel floating scan-panel" aria-label="Scans and inspection">
      <header className="panel-handle">
        <strong>Scans and inspection</strong>
        <IconButton icon="close" label="Close scans" onClick={props.onClose} />
      </header>
      <div className="panel-body">
        <div className="scan-add">
          <button title="Load a point cloud or mesh file from your computer as a scan" type="button" className="btn" onClick={() => fileInput.current?.click()} disabled={!!busy}>
            <Icon name="upload" /> Add a scan
          </button>
          <small>Points: XYZ, PLY, PCD, E57 · meshes: STL, OBJ, PLY, glTF, 3MF</small>
          <input ref={fileInput} type="file" style={{ display: "none" }} accept=".xyz,.asc,.pts,.csv,.txt,.ply,.pcd,.e57,.stl,.obj,.gltf,.glb,.3mf" onChange={(e) => (upload(e.target.files), (e.target.value = ""))} />
        </div>

        <PhotoScan active={active} onJob={props.onJob} onScan={(id) => void refresh(id)} notify={props.notify} />

        {props.onReferencePhotos && (
          <section className="scan-section">
            <h4>Reference photos → model</h4>
            <p className="scan-facts">A few separate views (side, top, front) cannot be aligned into a scan. Place each on its plane as a canvas, size it, and model over it; the model is checked against every view.</p>
            <div className="scan-actions">
              <button type="button" className="btn" onClick={props.onReferencePhotos}>
                <Icon name="image" /> Place reference photos…
              </button>
            </div>
          </section>
        )}

        {scans.length > 0 && (
          <div className="scan-list" role="listbox" aria-label="Scans">
            {scans.map((s) => (
              <button title={s.id === active ? `Hide ${s.name}` : `Show ${s.name} and its tools`} type="button" role="option" aria-selected={s.id === active} key={s.id} className={`list-row ${s.id === active ? "active" : ""}`} onClick={() => setActive(s.id === active ? null : s.id)}>
                <Icon name={s.kind === "mesh" ? "cube" : "scan"} />
                <span>
                  {s.name}
                  <small className="row-count">{s.kind === "mesh" ? `${s.triangles.toLocaleString()} triangles` : `${s.points.toLocaleString()} points`}</small>
                </span>
              </button>
            ))}
          </div>
        )}

        {entry && (
          <>
            <p className="scan-facts">
              {entry.bounds
                .slice(3)
                .map((v, i) => r3(v - entry.bounds[i]))
                .join(" × ")}{" "}
              mm · {entry.format.toUpperCase()}
              {entry.source !== "file" && ` · from ${entry.source}`}
              {view && view.step > 1 && ` · showing every ${view.step}th point`}
            </p>

            {entry.kind === "mesh" ? (
              <section className="scan-section">
                <h4>Mesh</h4>
                <div className="scan-actions">
                  <button title="Look for holes, flipped or crossing triangles and other defects, and mark them on the mesh" type="button" className="btn" onClick={check} disabled={!!busy}>
                    Check
                  </button>
                  <button title="Fix holes, flipped and duplicate triangles to make a closed solid (as a new scan)" type="button" className="btn" onClick={() => derive("Repair", "mesh/repair")} disabled={!!busy}>
                    Repair
                  </button>
                  <button title="Smooth out scanner noise on the mesh (10 passes, as a new scan)" type="button" className="btn" onClick={() => derive("Smooth", "mesh/smooth", { iterations: 10 })} disabled={!!busy}>
                    Smooth
                  </button>
                  <button title="Rebuild the mesh with even, well-shaped triangles (as a new scan)" type="button" className="btn" onClick={() => derive("Remesh", "mesh/remesh", {})} disabled={!!busy}>
                    Remesh
                  </button>
                </div>
                <div className="scan-actions">
                  <label>
                    Keep
                    <input type="number" min={0.01} max={1} step={0.05} value={ratio} onChange={(e) => setRatio(Number(e.target.value))} />
                  </label>
                  <button title="Cut the triangle count to the Keep fraction, staying close to the shape (as a new scan)" type="button" className="btn" onClick={() => derive("Decimate", "mesh/decimate", { ratio })} disabled={!!busy}>
                    Decimate
                  </button>
                  <button title="Sample 50,000 points from the mesh surface (as a new point scan)" type="button" className="btn" onClick={() => derive("Points", "surface/sample", { count: 50000 })} disabled={!!busy}>
                    To points
                  </button>
                </div>
                {report && (
                  <div className={`scan-report ${report.solid ? "ok" : "bad"}`}>
                    <strong>{report.solid ? "A closed, clean solid" : "Not a closed solid"}</strong>
                    <div className="scan-counts">
                      {counts.map(([label, n, color]) => (
                        <span key={label} className={n && label !== "Pieces" ? "has" : ""} style={{ "--mark": color } as React.CSSProperties}>
                          <b>{n.toLocaleString()}</b>
                          {label}
                        </span>
                      ))}
                    </div>
                    <small>
                      {report.vertices.toLocaleString()} vertices, {report.triangles.toLocaleString()} triangles · {r3(report.volume / 1000)} cm³
                    </small>
                  </div>
                )}
              </section>
            ) : (
              <section className="scan-section">
                <h4>Points</h4>
                <div className="scan-actions">
                  <button title="Thin the points to an even spacing (as a new scan)" type="button" className="btn" onClick={() => derive("Thinned", "points/downsample", {})} disabled={!!busy}>
                    Thin
                  </button>
                  <button title="Remove stray points far from the rest (as a new scan)" type="button" className="btn" onClick={() => derive("Cleaned", "points/outliers", {})} disabled={!!busy}>
                    Remove strays
                  </button>
                  <button title="Work out which way the surface faces at each point (needed to reconstruct)" type="button" className="btn" onClick={() => derive("Normals", "points/normals", {})} disabled={!!busy}>
                    Normals
                  </button>
                  <button title="Build a surface mesh through the points (as a new scan)" type="button" className="btn" onClick={() => derive("Surface", "surface/reconstruct", { depth: 7 })} disabled={!!busy}>
                    Reconstruct
                  </button>
                </div>
              </section>
            )}

            <section className="scan-section">
              <h4>Reverse engineer</h4>
              <div className="scan-actions">
                <button title="Find the plane, sphere, cylinder or other simple shape that best matches the scan" type="button" className="btn" onClick={fit} disabled={!!busy}>
                  Fit a primitive
                </button>
                <button title="Turn the scan into a part in the model" type="button" className="btn" onClick={makePart} disabled={!!busy}>
                  Make a part
                </button>
              </div>
            </section>

            <section className="scan-section">
              <h4>Inspect against a part</h4>
              <div className="scan-inspect">
                <select value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Part to compare with">
                  <option value="">Part…</option>
                  {parts.map((p) => (
                    <option key={p.id} value={p.id}>
                      {partName(p)}
                    </option>
                  ))}
                </select>
                <label>
                  ±
                  <input type="number" min={0.001} step={0.05} value={tolerance} onChange={(e) => setTolerance(Number(e.target.value))} aria-label="Tolerance (mm)" />
                  mm
                </label>
                <select value={align} onChange={(e) => setAlign(e.target.value as typeof align)} aria-label="Alignment">
                  <option value="none">As placed</option>
                  <option value="icp">Best fit</option>
                  <option value="best">Rough + best fit</option>
                </select>
                <button title="Colour the scan by how far it lies from the chosen part, and pass or fail it on the tolerance" type="button" className="btn primary" onClick={inspect} disabled={!!busy || !target}>
                  Inspect
                </button>
              </div>
              {inspection && (
                <div className={`scan-verdict ${inspection.passed ? "ok" : "bad"}`}>
                  <div className="verdict-head">
                    <span className="badge">{inspection.passed ? "PASS" : "FAIL"}</span>
                    <strong>
                      {(inspection.within * 100).toFixed(1)}% within ±{inspection.tolerance} mm
                    </strong>
                    <small>{inspection.stats.count.toLocaleString()} points</small>
                  </div>
                  <div className="dev-legend" aria-label="Deviation colours">
                    <span className="bar" style={{ background: `linear-gradient(90deg, ${DEVIATION_STOPS.join(", ")})` }} />
                    <span className="ticks">
                      <i>{signed(-Math.max(Math.abs(inspection.stats.min), Math.abs(inspection.stats.max), inspection.tolerance * 2))}</i>
                      <i>−{inspection.tolerance}</i>
                      <i>0</i>
                      <i>+{inspection.tolerance}</i>
                      <i>{signed(Math.max(Math.abs(inspection.stats.min), Math.abs(inspection.stats.max), inspection.tolerance * 2))}</i>
                    </span>
                  </div>
                  <div className="histogram" aria-label="Deviation histogram">
                    {inspection.histogram.map((n, i) => (
                      <span key={i} style={{ height: `${(n / Math.max(...inspection.histogram, 1)) * 100}%` }} className={i < 6 ? "under" : i > 9 ? "over" : "in"} />
                    ))}
                  </div>
                  <dl className="scan-stats">
                    <dt>Min</dt>
                    <dd>{signed(inspection.stats.min)}</dd>
                    <dt>Max</dt>
                    <dd>{signed(inspection.stats.max)}</dd>
                    <dt>Mean</dt>
                    <dd>{signed(inspection.stats.mean)}</dd>
                    <dt>RMS</dt>
                    <dd>{r3(inspection.stats.rms)}</dd>
                    <dt>σ</dt>
                    <dd>{r3(inspection.stats.std)}</dd>
                    <dt>95% within</dt>
                    <dd>±{r3(inspection.stats.p95)}</dd>
                    <dt>Over / under</dt>
                    <dd>
                      {inspection.over.toLocaleString()} / {inspection.under.toLocaleString()}
                    </dd>
                    {inspection.alignment && (
                      <>
                        <dt>Aligned</dt>
                        <dd>
                          RMS {r3(inspection.alignment.rms_before)} → {r3(inspection.alignment.rms_after)}
                        </dd>
                      </>
                    )}
                  </dl>
                  <button title="Add a requirement that keeps checking this part matches the scan within the tolerance" type="button" className="btn" onClick={requireIt} disabled={!!busy}>
                    <Icon name="shield" /> Keep checking (requirement)
                  </button>
                </div>
              )}
            </section>
          </>
        )}
        {busy && <p className="scan-busy">{busy}…</p>}
        {note && <p className="scan-note">{note}</p>}
        {error && <p className="scan-error">{error}</p>}
      </div>
    </aside>
  );
}
