import { useRef, useState } from "react";
import { sessionControls, type SessionSnapshot } from "./machineSessionState";

type Family = "jcz" | "ruida" | "grbl";
type Backend = "simulation" | "physical";
type Snapshot = SessionSnapshot;
type JsonObject = Record<string, unknown>;
const limit = 1024 * 1024;
const families: [Family, string][] = [["jcz", "JCZ fiber laser"], ["ruida", "Ruida CO₂ laser"], ["grbl", "GRBL CNC"]];
function download(name: string, value: unknown) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(value, null, 2) + "\n"], { type: "application/json" }));
  const link = document.createElement("a"); link.href = url; link.download = name; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function object(value: unknown): value is JsonObject { return !!value && typeof value === "object" && !Array.isArray(value); }
async function readFile(file: File): Promise<JsonObject> {
  if (file.size > limit - 256) throw new Error("Choose a JSON file smaller than 1 MiB.");
  const value: unknown = JSON.parse(await file.text());
  if (!object(value)) throw new Error("The file must contain a JSON object.");
  return value;
}

/** Explicit machine actions use one host for this computer, across all projects. */
export function MachineSession() {
  const [family, setFamily] = useState<Family>("jcz");
  const [backend, setBackend] = useState<Backend>("simulation");
  const [configuration, setConfiguration] = useState<JsonObject>({});
  const [configurationName, setConfigurationName] = useState("Default simulation");
  const [job, setJob] = useState<JsonObject | null>(null);
  const [jobName, setJobName] = useState("");
  const [hasHost, setHasHost] = useState(false);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [unknown, setUnknown] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [physicalReviewed, setPhysicalReviewed] = useState(false);
  const [startReviewed, setStartReviewed] = useState(false);
  const inFlight = useRef(false);
  const configInput = useRef<HTMLInputElement>(null);
  const jobInput = useRef<HTMLInputElement>(null);
  function resetSelection(nextFamily: Family, nextBackend: Backend) {
    setFamily(nextFamily); setBackend(nextBackend); setConfiguration({});
    setConfigurationName(nextBackend === "simulation" ? "Default simulation" : "No physical configuration selected");
    setJob(null); setJobName(""); setSnapshot(null); setPhysicalReviewed(false); setStartReviewed(false);
    setError(null); setNotice(null);
    if (configInput.current) configInput.current.value = "";
    if (jobInput.current) jobInput.current.value = "";
  }
  async function example() {
    if (inFlight.current) return;
    inFlight.current = true; setBusy("example"); setError(null);
    try {
      const response = await fetch("/machines/examples", { cache: "no-store", signal: AbortSignal.timeout(65000) });
      const result = await response.json();
      if (!response.ok || !object(result.examples?.[family])) throw new Error(result.error || "Example unavailable.");
      const sample = { family, backend: "simulation", configuration: {}, job: result.examples[family] };
      download(`${family}-simulation-session.json`, sample);
      if (!hasHost && backend === "simulation") {
        setConfiguration({}); setConfigurationName("Default simulation");
        setJob(result.examples[family]); setJobName(`${family} simulation example`);
        setNotice("Simulation example loaded and downloaded. Connect, prepare and start are separate actions.");
      }
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Example unavailable."); }
    finally { inFlight.current = false; setBusy(null); }
  }
  async function selectConfiguration(file: File | undefined) {
    if (!file || inFlight.current) return;
    inFlight.current = true; setBusy("loading configuration");
    setError(null); setPhysicalReviewed(false); setStartReviewed(false);
    try {
      const value = await readFile(file);
      if ("configuration" in value && "backend" in value) {
        if (value.backend !== "simulation" || value.family !== family || !object(value.configuration) || !object(value.job)) {
          throw new Error("Choose a simulation bundle for the selected family, or a raw physical configuration object.");
        }
        if (backend !== "simulation") throw new Error("Switch to simulation before loading a simulation bundle.");
        setConfiguration(value.configuration); setJob(value.job); setJobName(`${file.name} · job`);
      } else setConfiguration(value);
      setConfigurationName(file.name);
    } catch (cause) { setConfiguration({}); setConfigurationName("Invalid configuration"); setError(cause instanceof Error ? cause.message : "Invalid file."); }
    finally { inFlight.current = false; setBusy(null); }
  }
  async function selectJob(file: File | undefined) {
    if (!file || inFlight.current) return;
    inFlight.current = true; setBusy("loading job"); setJob(null);
    setError(null); setStartReviewed(false);
    try { setJob(await readFile(file)); setJobName(file.name); }
    catch (cause) { setJob(null); setJobName(""); setError(cause instanceof Error ? cause.message : "Invalid job file."); }
    finally { inFlight.current = false; setBusy(null); }
  }
  async function action(operation: string) {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(operation); setError(null); setNotice(null);
    const request: JsonObject = operation === "connect" ? { family, backend, configuration, ...(backend === "physical" ? { authorized: true } : {}) }
      : operation === "prepare" ? { job } : operation === "start" ? { authorized: true } : {};
    // Preserve an explicit Disconnect option after ambiguous connection failures.
    if (operation === "connect") setHasHost(true);
    try {
      const body = JSON.stringify(request);
      if (new TextEncoder().encode(body).length > limit - 128) throw new Error("Request exceeds the 1 MiB limit.");
      const response = await fetch(`/machines/session/${operation}`, { method: "POST", headers: { "Content-Type": "application/json" },
        body, cache: "no-store", signal: AbortSignal.timeout(65000) });
      const result = await response.json();
      if (object(result.result) && typeof result.result.state === "string") {
        const observed = result.result as Snapshot;
        setSnapshot(observed); setUnknown(!!observed.outcome_unknown || !!result.outcome_unknown);
        if (observed.connected) setHasHost(true);
        if (observed.family) setFamily(observed.family);
        if (observed.backend) setBackend(observed.backend);
      }
      if (result.outcome_unknown) setUnknown(true);
      if (result.host_disconnected || (operation === "close" && result.ok)) setHasHost(false);
      if (result.warning) setNotice(String(result.warning));
      if (!response.ok || result.ok !== true) throw new Error(result.error || "Machine operation failed.");
      if (operation === "start") setStartReviewed(false);
      if (operation === "connect") setUnknown(false);
      if (!result.warning) setNotice(`${operation === "status" ? "Status snapshot refreshed" : operation === "close" ? "Host disconnected" : `${operation[0].toUpperCase()}${operation.slice(1)} acknowledged`}.`);
    } catch (cause) {
      if (cause instanceof Error && (cause.name === "TimeoutError" || cause.name === "AbortError" || cause instanceof TypeError)) {
        setUnknown(true);
        setError("The browser lost the operation result. Machine outcome is unknown; do not repeat the command. Refresh status or disconnect explicitly.");
      } else setError(cause instanceof Error ? cause.message : "Machine operation failed.");
    } finally { inFlight.current = false; setBusy(null); }
  }
  const locked = !!busy || hasHost;
  const controls = sessionControls(snapshot, hasHost, unknown);
  const phase = snapshot?.state ?? (hasHost ? "connection outcome pending" : "disconnected");
  return <section aria-label="Machine session controls">
    <h3 className="panel-section">Machine session · this computer</h3>
    <p className="small muted">One connection is shared across projects. Simulation is the default and sends no hardware commands. Physical operation requires the attached computer’s machine configuration.</p>
    <label className="small">Controller family<br /><select disabled={locked} value={family} onChange={event => resetSelection(event.target.value as Family, backend)}>
      {families.map(([id, label]) => <option value={id} key={id}>{label}</option>)}
    </select></label>{" "}
    <label className="small">Connection<br /><select disabled={locked} value={backend} onChange={event => resetSelection(family, event.target.value as Backend)}>
      <option value="simulation">Simulation · no hardware</option><option value="physical">Physical machine</option>
    </select></label>
    <p className="small"><strong>{backend === "simulation" ? "SIMULATION" : "PHYSICAL MACHINE"}</strong> · {phase}. Machine setup remains unverified.</p>
    <p><button type="button" className="secondary" disabled={!!busy} onClick={example}>Download simulation example</button></p>
    <label className="small">Connection configuration or simulation bundle<br />
      <input ref={configInput} type="file" accept=".json,application/json" disabled={locked} onChange={event => void selectConfiguration(event.target.files?.[0])} />
    </label>
    <p className="small muted">{configurationName}. Uploading a file does not connect or execute.</p>
    {backend === "physical" && <>
      <p className="small muted">{family === "grbl" ? "Configuration requires port, baud and transport (posix or pyserial)."
        : family === "ruida" ? "Configuration requires profile, peer, bind_address and expected_card_id. Addresses must be numeric IPv4."
          : "Configuration requires profile, source_file, selector, identity and startup_review. File paths refer to the attached computer."}</p>
      <label className="small"><input type="checkbox" checked={physicalReviewed} disabled={locked} onChange={event => setPhysicalReviewed(event.target.checked)} /> I have reviewed this physical machine’s connection configuration.</label>
    </>}
    <p><button type="button" className="secondary" disabled={locked || (backend === "physical" && (!physicalReviewed || !Object.keys(configuration).length))} onClick={() => void action("connect")}>Connect {backend}</button>{" "}
      <button type="button" className="secondary" disabled={!!busy} onClick={() => void action("status")}>Refresh session status</button>{" "}
      <button type="button" className="secondary" disabled={!!busy || !controls.disconnect} onClick={() => void action("close")}>Disconnect</button></p>
    <label className="small">Job JSON file<br /><input ref={jobInput} type="file" accept=".json,application/json" disabled={!!busy || !controls.replaceJob} onChange={event => void selectJob(event.target.files?.[0])} /></label>
    {jobName && <p className="small muted">Selected: {jobName}</p>}
    <p><button type="button" className="secondary" disabled={!!busy || !controls.prepare || !job} onClick={() => void action("prepare")}>Prepare session job</button></p>
    {backend === "physical" && <label className="small"><input type="checkbox" checked={startReviewed} disabled={!!busy || !!snapshot?.worker_active} onChange={event => setStartReviewed(event.target.checked)} /> Machine area, material and job settings are checked; authorize this physical Start.</label>}
    <p>
      <button type="button" className="secondary" disabled={!!busy || !controls.start || (backend === "physical" && !startReviewed)} onClick={() => void action("start")}>Start {backend}</button>{" "}
      <button type="button" className="secondary" disabled={!!busy || !controls.pause} onClick={() => void action("pause")}>Pause</button>{" "}
      <button type="button" className="secondary" disabled={!!busy || !controls.resume} onClick={() => void action("resume")}>Resume</button>{" "}
      <button type="button" className="secondary" disabled={!!busy || !controls.stop} onClick={() => void action("abort")}>Stop job</button>
    </p>
    <div aria-live="polite">
      {busy && <p className="small">{busy} request in progress…</p>}
      {error && <p className="small" role="alert">{error}</p>}
      {notice && <p className="small">{notice}</p>}
      {(unknown || snapshot?.outcome_unknown) && <p className="small" role="alert"><strong>Machine outcome unknown.</strong> Host loss or termination does not confirm that hardware stopped. Commands are never automatically replayed.</p>}
      {snapshot?.error && <p className="small">Driver: {snapshot.error}</p>}
      {snapshot && <details><summary className="small">Session and driver evidence</summary><pre className="small" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{JSON.stringify(snapshot, null, 2)}</pre></details>}
    </div>
    <p className="small muted">Each connection accepts one job. Simulations can finish quickly; refresh status to see completion. Status refresh reads the host’s cached snapshot. The machine host runs independently of document changes, but ends with this engine server. Restarting never replays a job. Use the machine’s physical stop if host control is unavailable.</p>
  </section>;
}
