import { useRef, useState } from "react";

type QueryReport = {
  status: "queries_completed" | "failed" | "unavailable";
  device?: { vendor_id: string; product_id: string; bus: number; address: number };
  queries?: { name: string; response_hex?: string; response_words_le?: number[]; bytes_read?: number; status: string }[];
  errors?: string[];
  interpreted?: {
    board: { recognized: boolean; board_type: number | null };
    status: { ready: boolean; busy: boolean; paused: boolean } | null;
    position: { x: number; y: number; units: "scanner_counts" } | null;
  };
};
type Observation = {
  mode: "read_only_query";
  query_succeeded: boolean;
  framing_available: false;
  observed_at_ms: number;
  report: QueryReport;
};
type DriverCatalog = {
  mode: "offline_preparation";
  generated_at_utc: string;
  drivers: { id: string; name: string; summary: string; next_test: string;
    offline_checks: string; hardware_status: string; offline_test_count: number }[];
};

const boardName = (type: number | null) => ({ 1: "LMC2-Lite", 2: "LMC2-Lite-Digital", 3: "USB-Lite", 4: "LMC4", 5: "LMC4 family", 6: "LMC-Dynamic", 7: "USB-Lite dual-source" })[type as 1] ?? "Unknown";

const usbId = (value: string) => value.replace(/^0x/i, "").toUpperCase();

/** Machine ownership belongs to this engine's computer, not its document. */
export function MachineConnection() {
  const active = useRef(false);
  const [busy, setBusy] = useState(false);
  const [observation, setObservation] = useState<Observation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drivers, setDrivers] = useState<DriverCatalog | null>(null);
  const [driversBusy, setDriversBusy] = useState(false);
  const [driversError, setDriversError] = useState<string | null>(null);
  async function loadDrivers() {
    setDriversBusy(true);
    setDriversError(null);
    setDrivers(null);
    try {
      const response = await fetch("/machines/drivers", { cache: "no-store" });
      const result = await response.json();
      if (!response.ok || result.error) throw new Error(result.error || "Driver catalog unavailable.");
      if (result.mode !== "offline_preparation" || !Array.isArray(result.drivers)) throw new Error("Invalid driver catalog.");
      setDrivers(result as DriverCatalog);
    } catch (cause) {
      setDriversError(cause instanceof Error ? cause.message : "Driver catalog unavailable.");
    } finally {
      setDriversBusy(false);
    }
  }
  async function refresh() {
    if (active.current) return;
    active.current = true;
    setBusy(true);
    setError(null);
    // An old observation must not look like a fresh live connection on failure.
    setObservation(null);
    try {
      const response = await fetch("/machines/jcz/status", {
        method: "POST", headers: { Accept: "application/json" }, cache: "no-store",
      });
      const result = await response.json();
      if (!response.ok || result.error) throw new Error(result.error || "Controller query failed.");
      if (result.mode !== "read_only_query" || !result.report || typeof result.observed_at_ms !== "number") {
        throw new Error("The local engine returned an invalid controller report.");
      }
      setObservation(result as Observation);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Controller query failed.");
    } finally {
      active.current = false;
      setBusy(false);
    }
  }
  const report = observation?.report;
  const flags = report?.interpreted?.status;
  const position = report?.interpreted?.position;
  return (
    <section className="machine-connection" aria-label="Fiber laser connection">
      <h3 className="panel-section">Fiber laser · this computer</h3>
      <p className="small muted">Connect the EZCAD2 / JCZ controller by USB. Refresh reads its identity, version, status and position.</p>
      <button type="button" className="secondary" onClick={refresh} disabled={busy}>
        {busy ? "Checking controller…" : "Refresh controller"}
      </button>
      <div aria-live="polite">
        {error && <p className="small" role="alert">{error}</p>}
        {!observation && !error && !busy && <p className="small muted">Controller has not been checked.</p>}
        {observation && (
          <>
            <p className="small"><strong>{observation.query_succeeded ? "Controller replied" : "Controller query incomplete"}</strong>
              <br />Checked {new Date(observation.observed_at_ms).toLocaleTimeString()}. This is a snapshot.</p>
            {report?.device && <p className="small">USB {usbId(report.device.vendor_id)}:{usbId(report.device.product_id)}</p>}
            {report?.interpreted?.board.recognized && <p className="small">Controller family: {boardName(report.interpreted.board.board_type)}.</p>}
            {flags && <p className="small">Reported flags: {flags.ready ? "ready" : "not ready"}, {flags.busy ? "busy" : "not busy"}, {flags.paused ? "paused" : "not paused"}.</p>}
            {position && <p className="small">Scanner coordinates (raw): X {position.x}, Y {position.y}.</p>}
            {(flags || position) && <p className="small muted">Scanner coordinates are controller counts, not millimetres.</p>}
            {report?.errors?.map((message, i) => <p className="small" key={i}>{message}</p>)}
            {report?.queries && report.queries.length > 0 && (
              <details>
                <summary className="small">Controller replies</summary>
                {report.queries.map((query, i) => (
                  <p className="small" key={i}>{query.name}: {query.status}
                    {query.response_hex && <><br /><code>{query.response_hex}</code></>}
                  </p>
                ))}
              </details>
            )}
          </>
        )}
      </div>
      <p className="small muted">Framing is not available yet. Machine calibration and red-light configuration still need verification.</p>
      <h3 className="panel-section">Prepared machine drivers</h3>
      <p className="small muted">Review preparation for fiber, CO2 and CNC machines. This check does not connect to a machine.</p>
      <button type="button" className="secondary" onClick={loadDrivers} disabled={driversBusy}>
        {driversBusy ? "Loading drivers…" : "View prepared drivers"}
      </button>
      <div aria-live="polite">
        {driversError && <p className="small" role="alert">{driversError}</p>}
        {drivers?.drivers.map(driver => (
          <details key={driver.id}>
            <summary className="small">{driver.name}</summary>
            <p className="small">{driver.summary}</p>
            <p className="small muted">Offline checks: {driver.offline_checks === "passed" ? `${driver.offline_test_count} passed` : "verification pending"}.
              {" "}Machine validation: {driver.hardware_status === "partial" ? "partial" : "not performed"}.</p>
            <p className="small">Next machine check: {driver.next_test}</p>
          </details>
        ))}
      </div>
    </section>
  );
}
