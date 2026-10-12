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
  /** What the configuration shown leaves out (absent when nothing). */
  left_out?: LeftOut;
};

/** Parts a configuration leaves out, and the motion that went with them. */
export type LeftOut = {
  /** Parts left out, in the program's order. */
  parts?: string[];
  /** Every part the program made, left out or not (only when some are). */
  all?: string[];
  /** Parts it leaves out that the program no longer makes (renamed?). */
  missing?: string[];
  /** Hinges, joints, gear meshes, drives and couplings that went with them, in words. */
  dropped?: string[];
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

/** A triangle mesh in metres. Meshes from the engine arrive as typed arrays, shared by every entity showing the same content. */
export type MeshData = { positions: number[] | Float32Array; normals?: number[] | Float32Array; indices: number[] | Uint32Array; ref?: string };
/** How an entity view names its mesh on the wire, until it is fetched. */
type MeshRef = { ref: string; triangles: number };
const isRef = (m: unknown): m is MeshRef => !!m && typeof (m as MeshRef).ref === "string" && !("positions" in (m as object));

/** Reads one mesh from its binary form (see crates/server/src/meshes.rs). */
function decodeMesh(ref: string, buf: ArrayBuffer): MeshData {
  const head = new Uint32Array(buf, 0, 4);
  if (head[0] !== 0x31304d46) throw new Error(`mesh ${ref}: not a Forma mesh`);
  const [np, nn, ni] = [head[1], head[2], head[3]];
  let at = 16;
  const positions = new Float32Array(buf, at, np);
  at += np * 4;
  const normals = new Float32Array(buf, at, nn);
  at += nn * 4;
  const indices = new Uint32Array(buf, at, ni);
  return { positions, normals: nn ? normals : undefined, indices, ref };
}

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
  /** An instance showing a configuration of its shape: that configuration evaluated. */
  configured?: Configured;
  /** An instance: where it is now (where its joints put it when they hold it). */
  placed?: Placed;
  /** A spreadsheet: what each cell shows. */
  sheet?: SheetValues;
  /** A draft object where it is now: lines to draw, label, measured value. */
  draft?: DraftShape;
};

export type CommandInfo = { name: string; plugin: string; schema: unknown };
export type CellValue = { value?: number; text?: string; unit?: string; shown: string; formula?: string; error?: string };
export type SheetValues = { cells: Record<string, CellValue>; aliases: Record<string, string>; errors: number };
type V3 = [number, number, number];
export type DraftShape = {
  points: V3[];
  lines: V3[][];
  closed: boolean;
  length: number;
  area?: number;
  plane?: [V3, V3, V3];
  hatch?: [V3, V3][];
  label?: [V3, string];
  value?: number;
  arrows?: [V3, V3][];
};

/** Where an instance is in its parent: position, rotation (degrees about X, Y, Z) and the same as a quaternion. */
export type Placed = {
  at: [number, number, number];
  rotation: [number, number, number];
  quaternion: [number, number, number, number];
  /** The joint that places it (absent: it is free). */
  by?: string;
  /** The instance its joints hang from, when none reaches the ground. */
  anchor?: string;
  /** Every joint of its group, with the values it has now. */
  values?: Record<string, number[]>;
};

/** A shape evaluated in one of its configurations (what an instance naming it shows). */
export type Configured = { config: string; name: string; stats: ShapeStats; mesh: MeshData; mass: MassProperties; part_masses?: (number | null)[] };

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

/** A teammate's live cursor: what they have selected and where they point (part coordinates, mm). */
export type TeamCursor = { engine: string; name?: string; selected?: string | null; part?: string | null; pointer?: [number, number, number] | null; gone?: boolean };

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
  removed: { name: string; by: string; at: number }[];
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
/** What a project's name reads as when it has no title yet. */
export const prettyName = (name: string | null) => (name ? name.charAt(0).toUpperCase() + name.slice(1).replace(/-/g, " ") : "Default project");

/** The open project's title, description and tags (kept in the project; see `project.set_info`). */
export type ProjectInfo = { title: string; description: string; tags: string[] };
let openInfo: ProjectInfo | null = null;
export const projectInfo = () => openInfo;

/** A project's title: the open one's own, else its name made readable. */
export const projectTitle = (name: string | null = projectName) => (name === projectName && openInfo?.title ? openInfo.title : prettyName(name));

/** An engine URL scoped to the open project. */
export function projectUrl(path: string) {
  if (!projectName) return path;
  return `${path}${path.includes("?") ? "&" : "?"}project=${encodeURIComponent(projectName)}`;
}

export type ProjectEntry = { name: string; modified: number | null; revisions: number; default: boolean; info?: { title: string | null; description: string; tags: string[] } };

export type EngineError = { code: string; message: string };

export type Connection = "connecting" | "online" | "offline";

export type HistoryEntry = { revision: number; operation: string; user_id: string; at?: number };

/** One line of the Forma AI conversation, shared by every view of the project. */
export type ChatRecord = { role: "user" | "assistant" | "tool" | "error"; text: string; user?: string; at: number; attachments?: Attachment[] };

/** A picture (or PDF) sent to Forma AI with a message, kept with the conversation. */
export type Attachment = { id: string; kind: "png" | "jpg" | "gif" | "webp" | "pdf"; name: string; width?: number; height?: number };

/** Keeps a picture or PDF for the conversation; answers what to send with the message. */
export async function attachToAi(file: Blob, name: string): Promise<Attachment> {
  const res = await fetch(projectUrl(`/ai/attachments/${encodeURIComponent(name)}`), { method: "POST", body: file });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()) as Attachment;
}

/** Where an attachment can be shown from. */
export const attachmentUrl = (id: string) => projectUrl(`/ai/attachments/${id}`);

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
  /** Set when this session is read-only: a past version, or a view-only link. */
  readOnly: ReadOnlyInfo | null;
  /** Why the engine closed this session for good (a link turned off), if it did. */
  closedWhy: string | null;
};

/** What a read-only session shows. */
export type ReadOnlyInfo = { kind: "version" | "share"; label: string; title: string; version: string | null };

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
  /** `wsPath`: where the session is (default: this project's live session). */
  constructor(private wsPath?: string) {}
  private socket: WebSocket | null = null;
  private listeners = new Set<() => void>();
  private pending = new Map<number, Pending>();
  private nextRequest = 1;
  private retryDelay = 500;
  /** Meshes held, by content: fetched once, shared by every entity that shows one. */
  private meshes = new Map<string, MeshData>();
  /** Messages are taken in order; one naming meshes not held yet waits for them. */
  private inbox: Promise<void> = Promise.resolve();
  state: EngineState = {
    connection: "connecting",
    revision: 0,
    userId: sessionUser(),
    sessions: 0,
    entities: new Map(),
    history: [],
    lastChange: null,
    ai: { available: false, busy: false, chat: [], streaming: "", thinking: false },
    readOnly: null,
    closedWhy: null,
  };

  connect() {
    if (this.state.closedWhy) return;
    const protocol = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${protocol}://${location.host}${this.wsPath ?? projectUrl(`/ws?user=${encodeURIComponent(this.state.userId)}`)}`);
    this.socket = socket;
    this.update({ connection: "connecting" });
    socket.onopen = () => {
      this.retryDelay = 500;
    };
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      const missing = this.missingMeshes(message);
      // The common case (nothing to fetch) is handled at once, in order.
      if (!missing.length && this.idle) return this.receive(message);
      this.idle = false;
      const done = (this.inbox = this.inbox
        .then(() => this.fetchMeshes(missing))
        .then(
          () => this.receive(message),
          () => this.send({ type: "snapshot" }),
        )
        .finally(() => {
          if (this.inbox === done) this.idle = true;
        }));
    };
    socket.onclose = () => {
      for (const [, p] of this.pending) p.reject({ code: "DISCONNECTED", message: "Lost connection to the engine" });
      this.pending.clear();
      this.update({ connection: "offline" });
      setTimeout(() => this.connect(), this.retryDelay);
      this.retryDelay = Math.min(this.retryDelay * 2, 8000);
    };
  }

  private idle = true;

  /** Where meshes are fetched: this engine, or a view-only link's own path. */
  private meshUrl(ref: string) {
    const link = this.wsPath?.match(/^\/view\/([^/]+)\/ws/);
    return link ? `/view/${link[1]}/mesh/${ref}` : `/mesh/${ref}`;
  }

  /** Mesh refs a message names that are not held yet. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private missingMeshes(message: any): string[] {
    const list = message.type === "snapshot" || message.type === "changed" || message.type === "refreshed" ? (message.entities as EntityView[]) : null;
    if (!list) return [];
    const out = new Set<string>();
    for (const e of list) {
      for (const m of [e.mesh, e.configured?.mesh]) if (isRef(m) && !this.meshes.has(m.ref)) out.add(m.ref);
    }
    return [...out];
  }

  /** Fetches meshes (several at a time); each is kept by its ref. */
  private async fetchMeshes(refs: string[]) {
    let next = 0;
    const worker = async () => {
      while (next < refs.length) {
        const ref = refs[next++];
        if (this.meshes.has(ref)) continue;
        const r = await fetch(this.meshUrl(ref));
        if (!r.ok) throw new Error(`mesh ${ref}: ${r.status}`);
        this.meshes.set(ref, decodeMesh(ref, await r.arrayBuffer()));
      }
    };
    await Promise.all(Array.from({ length: Math.min(8, refs.length) }, worker));
  }

  /** Swaps mesh refs for the meshes held (one object per content, so equal meshes are the same object). */
  private resolveMeshes(list: EntityView[]) {
    for (const e of list) {
      if (isRef(e.mesh)) e.mesh = this.meshes.get(e.mesh.ref) ?? null;
      if (e.configured && isRef(e.configured.mesh)) e.configured = { ...e.configured, mesh: this.meshes.get(e.configured.mesh.ref)! };
    }
  }

  /** Lets go of meshes no entity shows any more. */
  private pruneMeshes() {
    if (this.meshes.size < 64) return;
    const used = new Set<string>();
    for (const e of this.state.entities.values()) {
      if (e.mesh?.ref) used.add(e.mesh.ref);
      if (e.configured?.mesh.ref) used.add(e.configured.mesh.ref);
    }
    for (const k of this.meshes.keys()) if (!used.has(k)) this.meshes.delete(k);
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
  shapePreview(target: string, values: Record<string, number>, finishes?: EdgeFinish[], config?: string): Promise<{ target: string; stats: ShapeStats; mesh: MeshData }> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject, retry: null });
      this.send({ type: "shape_preview", request_id: requestId, target, values, finishes, config });
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
  private cursorListeners = new Set<(c: TeamCursor) => void>();

  /** Tells teammates where this person is pointing and what is selected (live only, dropped when offline). */
  sendCursor(c: { selected: string | null; part: string | null; pointer: [number, number, number] | null }) {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify({ type: "cursor", ...c }));
  }

  /** Teammates' cursors as they move (and `gone` when one drops off). Returns the unsubscribe. */
  onCursor(listener: (c: TeamCursor) => void) {
    this.cursorListeners.add(listener);
    return () => {
      this.cursorListeners.delete(listener);
    };
  }

  /** Called when the team, its connections or its conflicts change. Returns the unsubscribe. */
  onTeam(listener: () => void) {
    this.teamListeners.add(listener);
    return () => {
      this.teamListeners.delete(listener);
    };
  }

  /** Sends a message to Forma AI, with what the person is looking at. */
  private viewListeners = new Set<(v: ViewCommand) => void>();

  /** The commands the engine accepts, with their parameter schemas (from the snapshot). */
  commands: CommandInfo[] = [];

  /** Shows something in this person's view, as Forma AI's view commands do (the console uses it). */
  emitView(v: ViewCommand) {
    for (const l of this.viewListeners) l(v);
  }

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
    attachments: string[] = [],
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const requestId = this.nextRequest++;
      this.pending.set(requestId, { resolve, reject, retry: null });
      this.send({ type: "ai_ask", request_id: requestId, text, context, attachments });
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
        this.resolveMeshes(message.entities);
        const entities = new Map<string, EntityView>();
        for (const e of message.entities as EntityView[]) entities.set(e.id, e);
        if (Array.isArray(message.commands)) this.commands = message.commands as CommandInfo[];
        this.update({
          connection: "online",
          revision: message.revision,
          sessions: message.sessions,
          entities,
          history: message.history as HistoryEntry[],
          ai: { ...this.state.ai, ...(message.ai ?? {}), streaming: "", thinking: false },
          readOnly: (message.read_only as ReadOnlyInfo | undefined) ?? null,
        });
        this.pruneMeshes();
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
        this.resolveMeshes(message.entities);
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
        this.resolveMeshes(message.entities);
        const entities = new Map(this.state.entities);
        for (const e of message.entities as EntityView[]) entities.set(e.id, e);
        this.update({ entities });
        break;
      }
      case "team":
      case "conflicts":
        for (const l of this.teamListeners) l();
        break;
      case "moved": {
        // This project was deleted (or closed) from the projects page: go where it went.
        const url = new URL(location.href);
        if (message.to) url.searchParams.set("project", message.to);
        else {
          url.searchParams.delete("project");
          url.searchParams.set("home", "1");
        }
        location.assign(url.toString());
        break;
      }
      case "cursor":
        for (const l of this.cursorListeners) l(message as TeamCursor);
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
        // A read-only session ends for good when its link is turned off.
        if (this.state.readOnly) this.update({ closedWhy: message.message });
        console.error("engine:", message.message);
        break;
    }
  }

  private update(patch: Partial<EngineState>) {
    this.state = { ...this.state, ...patch };
    if (patch.entities) {
      const e = [...patch.entities.values()].find((x) => x.components["core.project"]);
      openInfo = (e?.components["core.project"] as ProjectInfo | undefined) ?? null;
    }
    for (const listener of this.listeners) listener();
  }
}

export const mm = (value: number): Quantity => ({ value, unit: "mm" });
