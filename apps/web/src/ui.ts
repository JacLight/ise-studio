// Per-viewer interface preferences. Browser storage is a convenience only;
// the project itself lives in the engine.

export type AiMode = "mini" | "floating" | "docked" | "expanded" | "hidden";
export type FullMode = "floating" | "docked" | "expanded";
export type Group = "project" | "share" | "tools" | "view";
export type PanelKind = "project" | "components" | "objects" | "properties" | "requirements" | "history" | "views" | "comments";
export type Message = { role: "user" | "assistant" | "tool" | "error"; text: string; user?: string; attachments?: import("./engine").Attachment[] };
export type Point = { x: number; y: number };
/** A window edge a panel can dock to. */
export type DockSide = "left" | "right" | "bottom";

/** Docked panels: which tool panels dock (and to which edge), which of those are
 * open, where the BOM workbench and the AI sit, sizes (px: each column's width
 * as `col-left` / `col-right`, the bottom dock's height), and in a column the
 * panels' order top to bottom, their shares of its height, and which are folded. */
export type DockPrefs = {
  panels: Partial<Record<PanelKind, "left" | "right">>;
  open: PanelKind[];
  bench: DockSide | "float";
  ai: "left" | "right";
  sizes: Record<string, number>;
  order: string[];
  weights: Record<string, number>;
  collapsed: string[];
};

export type UiPrefs = {
  hidden: Partial<Record<Group, boolean>>;
  aiMode: AiMode;
  lastMode: FullMode;
  aiPosition: Point | null;
  messages: Message[];
  draft: string;
  voice: boolean;
  dock: DockPrefs;
};

const KEY = "ise.ui.v1";

const DEFAULTS: UiPrefs = {
  hidden: {},
  aiMode: "mini",
  lastMode: "floating",
  aiPosition: null,
  messages: [],
  draft: "",
  voice: false,
  dock: { panels: {}, open: [], bench: "bottom", ai: "right", sizes: {}, order: [], weights: {}, collapsed: [] },
};

/** The AI conversation belongs to the project it was about. */
const messagesKey = () => `${KEY}.messages.${new URLSearchParams(location.search).get("project") || "default"}`;

export function loadPrefs(): UiPrefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!saved || typeof saved !== "object") return DEFAULTS;
    const prefs = { ...DEFAULTS, ...saved } as UiPrefs;
    const messages = JSON.parse(localStorage.getItem(messagesKey()) ?? "null");
    // Older saves kept one conversation for every project: it stays with the default project.
    prefs.messages = Array.isArray(messages) ? messages : messagesKey().endsWith(".default") ? (prefs.messages ?? []) : [];
    if (!["mini", "floating", "docked", "expanded", "hidden"].includes(prefs.aiMode)) prefs.aiMode = "mini";
    if (!["floating", "docked", "expanded"].includes(prefs.lastMode)) prefs.lastMode = "floating";
    const dock = (saved.dock && typeof saved.dock === "object" ? saved.dock : {}) as Partial<DockPrefs>;
    const sides = ["left", "right", "bottom"];
    prefs.dock = {
      panels: Object.fromEntries(Object.entries(dock.panels ?? {}).filter(([, s]) => s === "left" || s === "right")),
      open: Array.isArray(dock.open) ? dock.open.filter((k) => typeof k === "string") : [],
      bench: dock.bench === "float" || sides.includes(dock.bench as string) ? (dock.bench as DockPrefs["bench"]) : "bottom",
      ai: dock.ai === "left" ? "left" : "right",
      sizes: Object.fromEntries(Object.entries(dock.sizes ?? {}).filter(([, n]) => typeof n === "number" && Number.isFinite(n))),
      order: Array.isArray(dock.order) ? dock.order.filter((k) => typeof k === "string") : [],
      weights: Object.fromEntries(Object.entries(dock.weights ?? {}).filter(([, n]) => typeof n === "number" && Number.isFinite(n) && n > 0)),
      collapsed: Array.isArray(dock.collapsed) ? dock.collapsed.filter((k) => typeof k === "string") : [],
    };
    return prefs;
  } catch {
    return DEFAULTS;
  }
}

export function savePrefs(prefs: UiPrefs) {
  try {
    const { messages, ...rest } = prefs;
    localStorage.setItem(KEY, JSON.stringify(rest));
    localStorage.setItem(messagesKey(), JSON.stringify(messages.slice(-60)));
  } catch {
    // Storage unavailable (private window, blocked site data): keep working without it.
  }
}
