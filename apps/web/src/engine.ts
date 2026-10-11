// Connection to the engine server. Holds the project state the UI renders and
// applies live changes from every session, including this one.

import type { EdgeFinish } from "./model";

export type Status = { state: "current" | "stale" | "computing" | "failed" | "blocked"; reason?: string };

export type MassProperties = { volume_m3: number; mass_kg: number | null; center_of_mass_m: [number, number, number] };

export type Fix = { label: string; operation: string; parameters: Record<string, unknown> };

export type RequirementResult = {
  passed: boolean;
  measured: number;
  limit: number;
  unit: string;
  upper_bound: boolean;
  summary: string;
  violations: { parts: string[]; names?: string[]; region: number[] | null }[];
  fix: Fix | null;
};

export type Finding = {
  code: string;
  severity: "error" | "warning" | "info";
  entities: string[];
  message: string;
  fix: Fix | null;
};

export type CheckReport = {
  ok: boolean;
  revision: number;
  requirements_total: number;
  requirements_passing: number;
  findings: Finding[];
};

export type ShapeStats = {
  volume_mm3: number;
  area_mm2: number;
  bounds_min: [number, number, number];
  bounds_max: [number, number, number];
  centroid: [number, number, number];
  genus: number;
  triangles: number;
  metrics?: Record<string, { value: number; unit: string }>;
  /** The program declares how its parts move (hinges, gear meshes). */
  moves?: boolean;
  /** How its parts turn: each hinged body, its axis, and turns per turn of the driving body. */
  motion?: Motion;
  /** The program joins parts with joint(): a linkage the engine solves. */
  linkage?: boolean;
  /** For assemblies: every fabricable part, each owning a range of the mesh's indices. */
  parts?: PartStats[];
};

export type Motion = {
  driver: number;
  bodies: { parts: string[]; at: [number, number, number]; axis: [number, number, number]; speed: number }[];
};

export type PartStats = {
  name: string;
  material: string | null;
  process: string | null;
  volume_mm3: number;
  bounds_min: [number, number, number];
  bounds_max: [number, number, number];
  centroid: [number, number, number];
  index_start: number;
  index_count: number;
};

export type MeshData = { positions: number[]; normals?: number[]; indices: number[] };

export type EntityView = {
  id: string;
  components: Record<string, unknown>;
  status: Status | null;
  mass: MassProperties | null;
  mesh: MeshData | null;
  result: RequirementResult | null;
  shape: ShapeStats | null;
  /** Mass (kg) of each assembly part, in the order of shape.parts. */
  part_masses?: (number | null)[];
  /** Totals for an assembly definition or instance (its parent's coordinates, mm). */
  assembly?: AssemblyStats | null;
  /** Parameter values after formulas, and the formulas (in names) that drive linked ones. */
  params?: Record<string, number>;
  formulas?: Record<string, string>;
};

/** A parameter that moves a face: the face moves `rate` mm per unit of it. */
export type FaceDriver = { key: string; label: string; value: number; rate: number };

/** One edge of a shape, for picking: a polyline, and what it is. */
export type PickEdge = {
  kind: "line" | "circle" | "curve";
  points: [number, number, number][];
  /** Lines: direction. Circles: axis. */
  dir?: [number, number, number];
  center?: [number, number, number];
  radius?: number;
};
/** The edges of one part (or the whole solid, part null) with its bounds [min x y z, max x y z]. */
export type PartEdges = { part: string | null; bounds: [number, number, number, number, number, number]; edges: PickEdge[] };

/** The project's team: members, live connections to their engines, and clashes. */
export type TeamStatus = {
  engine: string;
  my_name: string;
  public_url: string | null;
  addresses: string[];
  has_team: boolean;
  joining: boolean;
  members: { id: string; name: string; email: string; role: "editor" | "viewer"; joined: boolean; me: boolean; online: boolean }[];
  online: { engine: string; name: string; address: string; direction: string; since: number }[];
  waiting: number;
  changes: number;
  engines: number;
  conflicts: {
    kept: "ours" | "theirs";
    id: string;
    kind: string;
    path: string;
    ours: unknown;
    theirs: unknown;
    by: { user_id: string };
    command_id: string;
  }[];
};

export type AssemblyStats = {
  bounds_min: [number, number, number];
  bounds_max: [number, number, number];
  mass_kg: number | null;
  shapes: number;
  parts: number;
  metrics: Record<string, number>;
};

export type Quantity = { value: number; unit: string };

/** The open project: `?project=<name>` in the address, or the engine's default. */
export const projectName = new URLSearchParams(location.search).get("project") ?? "";

/** "miami-tower" → "Miami tower"; the default project reads "Default project". */
export const projectTitle = (name = projectName) => (name ? name.charAt(0).toUpperCase() + name.slice(1).replace(/-/g, " ") : "Default project");

/** An engine URL scoped to the open project. */
export function projectUrl(path: string) {
  if (!projectName) return path;
  return `${path}${path.includes("?") ? "&" : "?"}project=${encodeURIComponent(projectName)}`;
}

export type ProjectInfo = { name: string; modified: number | null; revisions: number; default: boolean };

export type EngineError = { code: string; message: string };

export type Connection = "connecting" | "online" | "offline";

export type HistoryEntry = { revision: number; operation: string; user_id: string; at?: number };

/** One line of the Forma AI conversation, shared by every view of the project. */
export type ChatRecord = { role: "user" | "assistant" | "tool" | "error"; text: string; user?: string; at: number };

/** A view command (from Forma AI) for this person: select, frame, isolate, hide, paint, camera. */
export type ViewCommand = { action: string; target: string | null; value: string | null };

export type AiState = {
  /** A Claude session can run on this computer. */
  available: boolean;
  busy: boolean;
  chat: ChatRecord[];
  /** The reply being written right now. */
  streaming: string;
  thinking: boolean;
};

export type EngineState = {
  connection: Connection;
  revision: number;
  userId: string;
  sessions: number;
  entities: ReadonlyMap<string, EntityView>;
  /** Newest first. */
  history: HistoryEntry[];
  lastChange: { revision: number; operation: string; byMe: boolean } | null;
  ai: AiState;
};

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: EngineError) => void;
  retry: (() => void) | null;
};

function sessionUser(): string {
  const make = () => `user-${Math.random().toString(36).slice(2, 7)}`;
  try {
    const saved = sessionStorage.getItem("forma.user");
    if (saved) return saved;
    const user = make();
    sessionStorage.setItem("forma.user", user);
    return user;
  } catch {
    return make();
  }
}

export class Engine {
  private socket: WebSocket | null = null;
  private listeners = new Set<() => void>();
  private pending = new Map<number, Pending>();
  private nextRequest = 1;
  private retryDelay = 500;
  state: EngineState = {
    connection: "connecting",
    revision: 0,
    userId: sessionUser(),
    sessions: 0,
    entities: new Map(),
    history: [],
    lastChange: null,
    ai: { available: false, busy: false, chat: [], streaming: "", thinking: false },
  };

  connect() {
    const protocol = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${protocol}://${location.host}${projectUrl(`/ws?user=${encodeURIComponent(this.state.userId)}`)}`);
    this.socket = socket;
    this.update({ connection: "connecting" });
    socket.onopen = () => {
      this.retryDelay = 500;
    };
    socket.onmessage = (event) => this.receive(JSON.parse(event.data));
    socket.onclose = () => {
      for (const [, p] of this.pending) p.reject({ code: "DISCONNECTED", message: "Lost connection to the engine" });
      this.pending.clear();
      this.update({ connection: "offline" });
      setTimeout(() => this.connect(), this.retryDelay);
      this.retryDelay = Math.min(this.retryDelay * 2, 8000);
    };
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = () => this.state;

  /** Commits a command. If another session committed first, retries once on the new revision. */
  commit(operation: string, parameters: Record<string, unknown>): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const send = (allowRetry: boolean) => {
        const requestId = this.nextRequest++;
        this.pending.set(requestId, { resolve, reject, retry: allowRetry ? () => send(false) : null });
        this.send({
          type: "commit",
          request_id: requestId,
          command_id: crypto.randomUUID(),
          operation,
          parameters,
          expected_revision: this.state.revision,
        });
      };
      send(true);
    });
  }

  /** Validates a command against the current revision without changing anything. */
  preview(operation: string, parameters: Record<string, unknown>): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve, reject, retry: null });
      this.send({ type: "preview", request_id: requestId, operation, parameters, expected_revision: this.state.revision });
    });
  }

  /** Evaluates a shape with changed parameters without committing (for live dragging). */
  shapePreview(target: string, values: Record<string, number>, finishes?: EdgeFinish[]): Promise<{ target: string; stats: ShapeStats; mesh: MeshData }> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject, retry: null });
      this.send({ type: "shape_preview", request_id: requestId, target, values, finishes });
    });
  }

  /** Which parameter drives the flat face at a point (program coordinates), if any. */
  shapeFace(target: string, part: string | null, point: [number, number, number], normal: [number, number, number]): Promise<{ driver: FaceDriver | null }> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject, retry: null });
      this.send({ type: "shape_face", request_id: requestId, target, part, point, normal });
    });
  }

  /** A shape's edges as it is now, for picking (program coordinates, mm). */
  shapeEdges(target: string): Promise<{ target: string; parts: PartEdges[] }> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject, retry: null });
      this.send({ type: "shape_edges", request_id: requestId, target });
    });
  }

  /** The engine's check gate: requirements, part health and overlaps. */
  check(): Promise<CheckReport> {
    return this.request("check") as Promise<CheckReport>;
  }

  private teamListeners = new Set<() => void>();

  /** Called when the team, its connections or its conflicts change. Returns the unsubscribe. */
  onTeam(listener: () => void) {
    this.teamListeners.add(listener);
    return () => {
      this.teamListeners.delete(listener);
    };
  }

  /** Sends a message to Forma AI, with what the person is looking at. */
  private viewListeners = new Set<(v: ViewCommand) => void>();

  /** Follow view commands addressed to this person. Returns the unsubscribe. */
  onView(listener: (v: ViewCommand) => void) {
    this.viewListeners.add(listener);
    return () => {
      this.viewListeners.delete(listener);
    };
  }

  askAi(
    text: string,
    context: { selected: string | null; picked_part: string | null; isolated?: string | null; hidden?: string[]; color_by?: string },
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve, reject, retry: null });
      this.send({ type: "ai_ask", request_id: requestId, text, context });
    });
  }

  stopAi(): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve, reject, retry: null });
      this.send({ type: "ai_stop", request_id: requestId });
    });
  }

  undo() {
    return this.request("undo");
  }

  redo() {
    return this.request("redo");
  }

  private request(type: "undo" | "redo" | "check"): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve, reject, retry: null });
      this.send({ type, request_id: requestId });
    });
  }

  private send(message: object) {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(message));
    } else {
      queueMicrotask(() => {
        for (const [id, p] of this.pending) {
          this.pending.delete(id);
          p.reject({ code: "DISCONNECTED", message: "Not connected to the engine" });
        }
      });
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private receive(message: any) {
    switch (message.type) {
      case "snapshot": {
        const entities = new Map<string, EntityView>();
        for (const e of message.entities as EntityView[]) entities.set(e.id, e);
        this.update({
          connection: "online",
          revision: message.revision,
          sessions: message.sessions,
          entities,
          history: message.history as HistoryEntry[],
          ai: { ...this.state.ai, ...(message.ai ?? {}), streaming: "", thinking: false },
        });
        break;
      }
      case "view": {
        // Only the person the AI is talking to (or this person's own command).
        if (message.to && message.to !== this.state.userId) break;
        for (const l of this.viewListeners) l({ action: message.action, target: message.target ?? null, value: message.value ?? null });
        break;
      }
      case "ai": {
        const ai = this.state.ai;
        switch (message.event) {
          case "message":
            // A finished block replaces what was streaming.
            this.update({ ai: { ...ai, chat: [...ai.chat, message.record as ChatRecord].slice(-300), streaming: message.record.role === "assistant" ? "" : ai.streaming, thinking: false } });
            break;
          case "delta":
            this.update({ ai: { ...ai, streaming: ai.streaming + message.text, thinking: false } });
            break;
          case "thinking":
            if (!ai.thinking) this.update({ ai: { ...ai, thinking: true } });
            break;
          case "status":
            this.update({ ai: { ...ai, busy: message.busy } });
            break;
          case "done":
            this.update({ ai: { ...ai, busy: false, streaming: "", thinking: false } });
            break;
        }
        break;
      }
      case "changed": {
        const entities = new Map(this.state.entities);
        for (const e of message.entities as EntityView[]) entities.set(e.id, e);
        for (const id of message.removed as string[]) entities.delete(id);
        this.update({
          revision: message.revision,
          entities,
          history: [
            { revision: message.revision, operation: message.operation, user_id: message.actor.user_id, at: Date.now() },
            ...this.state.history,
          ].slice(0, 200),
          lastChange: {
            revision: message.revision,
            operation: message.operation,
            byMe: message.actor.user_id === this.state.userId,
          },
        });
        break;
      }
      case "presence":
        this.update({ sessions: message.sessions });
        break;
      case "refreshed": {
        // Recomputed in place (an imported file arrived): no new revision.
        const entities = new Map(this.state.entities);
        for (const e of message.entities as EntityView[]) entities.set(e.id, e);
        this.update({ entities });
        break;
      }
      case "team":
      case "conflicts":
        for (const l of this.teamListeners) l();
        break;
      case "resync":
        this.send({ type: "snapshot" });
        break;
      case "result": {
        const p = this.pending.get(message.request_id);
        if (!p) return;
        this.pending.delete(message.request_id);
        if (message.ok) p.resolve(message.value);
        else if (message.error.code === "STALE_REVISION" && p.retry) p.retry();
        else p.reject(message.error);
        break;
      }
      case "error":
        console.error("engine:", message.message);
        break;
    }
  }

  private update(patch: Partial<EngineState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
}

export const mm = (value: number): Quantity => ({ value, unit: "mm" });
