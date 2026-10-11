import { useRef, useState } from "react";

type Family = "jcz" | "ruida" | "delta_nc5" | "grbl";
type Artifact = { name: string; mime_type: string; encoding: "base64" | "utf-8"; data: string; bytes: number; sha256: string };
type Prepared = { schema_version: 1; status: "prepared"; family: Family; hardware_commands_sent: 0;
  machine_ready: false; summary: Record<string, unknown>; artifacts: Artifact[]; warnings: string[] };
const families: [Family, string][] = [["jcz", "JCZ fiber laser"], ["ruida", "Ruida CO₂ laser"],
  ["delta_nc5", "Delta NC5 milling"], ["grbl", "GRBL G-code"]];
const inputLimit = 1024 * 1024;
function download(name: string, data: BlobPart, mime: string) {
  const url = URL.createObjectURL(new Blob([data], { type: mime }));
  const link = document.createElement("a"); link.href = url; link.download = name; link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function downloadArtifact(artifact: Artifact) {
  if (artifact.encoding === "utf-8") download(artifact.name, artifact.data, artifact.mime_type);
  else download(artifact.name, Uint8Array.from(atob(artifact.data), c => c.charCodeAt(0)), artifact.mime_type);
}
/** Offline preparation is independent of project edits and machine sessions. */
export function MachinePreparation() {
  const [family, setFamily] = useState<Family>("jcz");
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [exampleBusy, setExampleBusy] = useState(false);
  const active = useRef(false);
  const upload = useRef<HTMLInputElement>(null);
  async function example() {
    setExampleBusy(true); setError(null);
    try {
      const response = await fetch("/machines/examples", { cache: "no-store", signal: AbortSignal.timeout(65000) });
      const result = await response.json();
      if (!response.ok || !result.examples?.[family]) throw new Error(result.error || "Example unavailable.");
      download(`${family}-example.json`, JSON.stringify(result.examples[family], null, 2) + "\n", "application/json");
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Example unavailable."); }
    finally { setExampleBusy(false); }
  }
  async function prepare() {
    if (!file || active.current) return;
    active.current = true; setBusy(true); setError(null); setPrepared(null);
    try {
      if (file.size > inputLimit) throw new Error("Choose a JSON file smaller than 1 MiB.");
      const job: unknown = JSON.parse(await file.text());
      if (!job || typeof job !== "object" || Array.isArray(job)) throw new Error("The file must contain a job object.");
      const body = JSON.stringify({ schema_version: 1, family, job });
      if (new TextEncoder().encode(body).length > inputLimit) throw new Error("Job and profile exceed the 1 MiB limit.");
      const response = await fetch("/machines/prepare", { method: "POST", headers: { "Content-Type": "application/json" },
        body, cache: "no-store", signal: AbortSignal.timeout(65000) });
      const result = await response.json();
      if (!response.ok || result.status !== "prepared") throw new Error(result.error || "Job preparation failed.");
      if (result.schema_version !== 1 || result.family !== family || result.hardware_commands_sent !== 0
          || result.machine_ready !== false || !Array.isArray(result.artifacts) || !Array.isArray(result.warnings)) {
        throw new Error("Invalid offline preparation report.");
      }
      setPrepared(result as Prepared);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Job preparation failed."); }
    finally { active.current = false; setBusy(false); }
  }
  return <section aria-label="Offline machine job preparation">
    <h3 className="panel-section">Prepare a machine job</h3>
    <p className="small muted">Create downloadable job files on this computer. Preparation sends no machine commands and does not establish machine readiness.</p>
    <label className="small">Controller family<br />
      <select value={family} disabled={busy || exampleBusy} onChange={event => {
        setFamily(event.target.value as Family); setFile(null); setPrepared(null); setError(null);
        if (upload.current) upload.current.value = "";
      }}>{families.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>
    </label>
    <p className="small"><button type="button" className="secondary" onClick={example} disabled={busy || exampleBusy}>
      {exampleBusy ? "Loading example…" : "Download job example"}</button></p>
    <p className="small muted">Edit the example’s profile and geometry, then choose that JSON file. Delta NC5 uses its milling format; OPEN CNC live control needs a separate SDK.</p>
    <label className="small">Job JSON file (up to 1 MiB)<br />
      <input ref={upload} type="file" accept=".json,application/json" disabled={busy} onChange={event => {
        const selected = event.target.files?.[0] ?? null;
        setPrepared(null); setError(null);
        if (selected && selected.size > inputLimit) { setFile(null); setError("Choose a JSON file smaller than 1 MiB."); }
        else setFile(selected);
      }} />
    </label>
    <p><button type="button" className="secondary" disabled={!file || busy || exampleBusy} onClick={prepare}>
      {busy ? "Preparing job…" : "Prepare offline"}</button></p>
    <div aria-live="polite">
      {error && <p className="small" role="alert">{error}</p>}
      {prepared && <>
        <p className="small"><strong>Job files prepared.</strong> No machine commands sent. Machine compatibility and setup still require verification.</p>
        <ul>{prepared.artifacts.map(artifact => <li key={artifact.name}>
          <button type="button" className="secondary" onClick={() => downloadArtifact(artifact)}>Download {artifact.name}</button>
          <span className="small muted"> {artifact.bytes.toLocaleString()} bytes</span>
        </li>)}</ul>
        {prepared.warnings.map((warning, index) => <p className="small" key={index}>{warning}</p>)}
        <details><summary className="small">Preparation details</summary>
          <pre className="small" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{JSON.stringify(prepared.summary, null, 2)}</pre>
        </details>
      </>}
    </div>
  </section>;
}
