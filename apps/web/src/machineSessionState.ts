/** Cached session state only; this module has no network or machine side effects. */
export type SessionSnapshot = {
  family?: "jcz" | "ruida" | "grbl";
  backend?: "simulation" | "physical";
  state: string;
  connected: boolean;
  job_prepared: boolean;
  worker_active: boolean;
  outcome_unknown: boolean;
  error?: string | null;
  driver?: Record<string, unknown>;
  machine_ready?: boolean;
};
export function sessionControls(snapshot: SessionSnapshot | null, hasHost: boolean, unknown: boolean) {
  const available = hasHost && !unknown && !snapshot?.outcome_unknown && !!snapshot?.connected;
  return {
    prepare: available && snapshot.state === "connected" && !snapshot.worker_active,
    start: available && snapshot.state === "prepared" && snapshot.job_prepared && !snapshot.worker_active,
    pause: available && snapshot.state === "running" && snapshot.worker_active,
    resume: available && snapshot.state === "paused" && snapshot.worker_active,
    stop: available && ["connected", "prepared", "running", "paused"].includes(snapshot.state),
    disconnect: hasHost,
    replaceJob: !snapshot?.job_prepared && !snapshot?.worker_active,
  };
}
