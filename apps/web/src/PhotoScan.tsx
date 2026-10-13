// Photogrammetry in the Scans panel: photos of an object from many angles
// sent to the engine (Apple Object Capture or COLMAP), the job's progress
// as the engine reports it (cancel), the quality of the result (photos
// used, reprojection error, how it got its size, coverage gaps), a short
// capture guide with the marker sheet to print, and sizing a scan by a
// known distance.

import { useEffect, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";

type EngineInfo = { name: string; available: boolean; detail: string; version?: string; dense?: boolean };
type Coverage = { sectors: number[]; lowest_elevation: number; highest_elevation: number; gaps: string[] };
type Job = {
  id: string;
  name: string;
  engine: string;
  images: number;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  stage: string;
  progress: number;
  error?: string;
  result?: {
    scan: { id: string; name: string };
    clean?: { id: string; name: string };
    cleanError?: string;
    quality: { images: number; used: number; rejected: { image?: string; sample?: number; reason: string }[]; reprojectionPx: number | null; coverage: Coverage; holes?: number; advice: string[] };
    scale: { method: string; factor: number; uncertainty: number | null; detail: string };
  };
};

async function call<T>(method: string, path: string, body?: BodyInit): Promise<T> {
  const r = await fetch(projectUrl(path), { method, body });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

const ENGINE_NAMES: Record<string, string> = { "object-capture": "Apple Object Capture", colmap: "COLMAP" };

/** Twelve 30° sectors round the object, shaded by how many photos came from each. */
function CoverageRing({ c }: { c: Coverage }) {
  const top = Math.max(1, ...c.sectors);
  return (
    <svg className="pg-ring" viewBox="-50 -50 100 100" role="img" aria-label={`Photos per 30° sector: ${c.sectors.join(", ")}`}>
      {c.sectors.map((n, i) => {
        const a0 = ((i * 30 - 90) * Math.PI) / 180;
        const a1 = (((i + 1) * 30 - 90) * Math.PI) / 180;
        const [r0, r1] = [24, 46];
        const p = (r: number, a: number) => `${(r * Math.cos(a)).toFixed(2)} ${(r * Math.sin(a)).toFixed(2)}`;
        const d = `M ${p(r0, a0)} L ${p(r1, a0)} A ${r1} ${r1} 0 0 1 ${p(r1, a1)} L ${p(r0, a1)} A ${r0} ${r0} 0 0 0 ${p(r0, a0)} Z`;
        return <path key={i} d={d} className={n ? "" : "gap"} style={{ opacity: n ? 0.3 + (0.7 * n) / top : 1 }} />;
      })}
      <text y="4" textAnchor="middle">
        {c.sectors.reduce((a, b) => a + b, 0)}
      </text>
    </svg>
  );
}

export function PhotoScan(props: { active: string | null; onJob: (listener: (job: unknown) => void) => () => void; onScan: (id: string) => void; notify: (m: string) => void }) {
  const [engines, setEngines] = useState<EngineInfo[] | null>(null);
  const [engine, setEngine] = useState("auto");
  const [detail, setDetail] = useState("medium");
  const [mask, setMask] = useState(false);
  const [job, setJob] = useState<Job | null>(null);
  const [upload, setUpload] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scale, setScale] = useState({ from: "", to: "", distance: "" });
  const files = useRef<HTMLInputElement>(null);
  const onScan = useRef(props.onScan);
  onScan.current = props.onScan;

  useEffect(() => {
    call<{ engines: EngineInfo[] }>("GET", "/photogrammetry/engines")
      .then((r) => setEngines(r.engines))
      .catch((e) => setError(String(e.message ?? e)));
    // A job already running (started from ./forma or another view).
    call<Job[]>("GET", "/photogrammetry")
      .then((list) => setJob(list[0] ?? null))
      .catch(() => {});
  }, []);

  // Progress pushed by the engine over the project's connection.
  useEffect(
    () =>
      props.onJob((j) => {
        const next = j as Job;
        setJob((cur) => (!cur || cur.id === next.id || next.state === "running" ? next : cur));
        if (next.state === "done" && next.result) onScan.current((next.result.clean ?? next.result.scan).id);
      }),
    [props.onJob], // eslint-disable-line react-hooks/exhaustive-deps
  );

  const start = async (list: FileList | null) => {
    if (!list?.length) return;
    setError(null);
    const set = `photos-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-")}`;
    try {
      let n = 0;
      for (const f of Array.from(list)) {
        setUpload(`Uploading ${++n} of ${list.length}`);
        await call("POST", `/photos/${encodeURIComponent(set)}/${encodeURIComponent(f.name)}`, await f.arrayBuffer());
      }
      setUpload(null);
      const qs = new URLSearchParams({ set, engine, detail, ...(mask ? { mask: "1" } : {}) });
      setJob(await call<Job>("POST", `/photogrammetry?${qs}`));
    } catch (e) {
      setUpload(null);
      setError(String((e as Error).message ?? e));
    }
  };

  const cancel = async () => {
    if (job) await call("POST", `/photogrammetry/${job.id}/cancel`).catch((e) => setError(String(e.message ?? e)));
  };

  const transform = async (op: "scale" | "level") => {
    if (!props.active) return;
    setError(null);
    const v = (s: string) => s.split(/[\s,]+/).filter(Boolean).map(Number);
    const body = op === "scale" ? { from: v(scale.from), to: v(scale.to), distance: Number(scale.distance) } : {};
    try {
      const r = await call<{ scan: { id: string; name: string }; report: { detail: string } }>("POST", `/scans/${props.active}/transform/${op}`, JSON.stringify(body));
      props.notify(r.report.detail);
      props.onScan(r.scan.id);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
  };

  const running = job && (job.state === "queued" || job.state === "running");
  const r = job?.state === "done" ? job.result : undefined;
  const usable = engines?.filter((e) => e.available) ?? [];

  return (
    <section className="scan-section pg">
      <h4>Photos to model</h4>
      <div className="pg-options">
        <label>
          Engine
          <select value={engine} onChange={(e) => setEngine(e.target.value)} disabled={!!running}>
            <option value="auto">Best available</option>
            {engines?.map((e) => (
              <option key={e.name} value={e.name} disabled={!e.available}>
                {ENGINE_NAMES[e.name] ?? e.name}
                {e.available ? "" : " (not here)"}
              </option>
            ))}
          </select>
        </label>
        <label>
          Detail
          <select value={detail} onChange={(e) => setDetail(e.target.value)} disabled={!!running}>
            <option value="preview">Preview</option>
            <option value="reduced">Reduced</option>
            <option value="medium">Medium</option>
            <option value="full">Full</option>
          </select>
        </label>
        <label className="pg-check">
          <input type="checkbox" checked={mask} onChange={(e) => setMask(e.target.checked)} disabled={!!running} /> Object only (cut the background away)
        </label>
      </div>
      <div className="scan-actions">
        <button type="button" className="btn primary" onClick={() => files.current?.click()} disabled={!!running || !!upload || !usable.length}>
          <Icon name="upload" /> Choose photos…
        </button>
        <a className="btn" href="/photogrammetry/markers.pdf" target="_blank" rel="noreferrer">
          Marker sheet (PDF)
        </a>
        <input ref={files} type="file" multiple accept=".jpg,.jpeg,.png,.heic,.heif,.zip" style={{ display: "none" }} onChange={(e) => (start(e.target.files), (e.target.value = ""))} />
      </div>
      {engines && (
        <ul className="pg-engines">
          {engines.map((e) => (
            <li key={e.name} className={e.available ? "ok" : "missing"} title={e.version}>
              {e.detail}
            </li>
          ))}
        </ul>
      )}
      {upload && <p className="scan-busy">{upload}…</p>}
      {job && running && (
        <div className="pg-job" aria-live="polite">
          <div className="pg-bar">
            <span style={{ width: `${Math.round(job.progress * 100)}%` }} />
          </div>
          <div className="pg-job-line">
            <span>
              {Math.round(job.progress * 100)}% · {job.stage} · {job.images} photos, {ENGINE_NAMES[job.engine] ?? job.engine}
            </span>
            <button type="button" className="btn" onClick={cancel}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {job && (job.state === "failed" || job.state === "cancelled") && <p className="scan-error">{job.state === "cancelled" ? "Cancelled." : job.error}</p>}
      {r && (
        <div className="pg-quality">
          <CoverageRing c={r.quality.coverage} />
          <dl>
            <dt>Photos</dt>
            <dd>
              {r.quality.used} of {r.quality.images} used{r.quality.rejected.length ? `, ${r.quality.rejected.length} rejected` : ""}
            </dd>
            <dt>Reprojection</dt>
            <dd>{r.quality.reprojectionPx != null ? `${r.quality.reprojectionPx.toFixed(2)} px` : "—"}</dd>
            <dt>Size</dt>
            <dd title={r.scale.detail}>
              {r.scale.method === "none" ? "not scaled" : `${r.scale.method}${r.scale.uncertainty != null ? ` ±${(r.scale.uncertainty * 100).toFixed(2)} %` : ""}`}
            </dd>
            <dt>Views</dt>
            <dd>
              {Math.round(r.quality.coverage.lowest_elevation)}°–{Math.round(r.quality.coverage.highest_elevation)}° up
            </dd>
          </dl>
          {r.quality.advice.length > 0 && (
            <ul className="pg-advice">
              {r.quality.advice.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          )}
          {r.cleanError && <p className="scan-error">Cleaning: {r.cleanError}</p>}
        </div>
      )}
      {error && <p className="scan-error">{error}</p>}
      <details className="pg-guide">
        <summary>How to take the photos</summary>
        <ol>
          <li>Print the marker sheet at 100 % (check the 100 mm bar), lay it flat and stand the object in the middle. It gives the model its true size and floor.</li>
          <li>Take 30–80 sharp photos in two or three rings round it (low, about 30° and 60° up), each overlapping the next by two thirds. Keep some markers in every photo.</li>
          <li>Soft, even light that does not move; no flash. A turntable works if the sheet turns with the object and the background is plain.</li>
          <li>Shiny, clear or plain surfaces have nothing to match: dust them with scanning spray or chalk, or stick on a few bits of tape.</li>
          <li>Fill the frame with the object; do not zoom between photos. iPhone photos with depth (portrait mode) make Object Capture's size metric even without the sheet.</li>
        </ol>
      </details>
      {props.active && (
        <details className="pg-guide">
          <summary>Size this scan by a known distance</summary>
          <div className="pg-scale">
            <input placeholder="from x, y, z" value={scale.from} onChange={(e) => setScale({ ...scale, from: e.target.value })} />
            <input placeholder="to x, y, z" value={scale.to} onChange={(e) => setScale({ ...scale, to: e.target.value })} />
            <input placeholder="mm" value={scale.distance} onChange={(e) => setScale({ ...scale, distance: e.target.value })} />
            <button type="button" className="btn" onClick={() => transform("scale")} disabled={!scale.from || !scale.to || !scale.distance}>
              Scale
            </button>
            <button type="button" className="btn" onClick={() => transform("level")}>
              Level on floor
            </button>
          </div>
        </details>
      )}
    </section>
  );
}
