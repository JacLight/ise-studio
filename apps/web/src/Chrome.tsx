// Floating controls over the canvas. Each group can be closed; a closed group
// collapses into a small dots button in its place that brings it back.

import { type Connection, projectTitle } from "./engine";
import { Icon, IconButton } from "./icons";
import type { Group, PanelKind } from "./ui";
import type { ViewName } from "./Viewport";

const GROUP_LABELS: Record<Group, string> = {
  project: "project bar",
  share: "sharing controls",
  tools: "tools",
  view: "view bar",
};

export function Collapsed({ group, onShow }: { group: Group; onShow: () => void }) {
  return (
    <button
      type="button"
      className={`collapsed floating collapsed-${group}`}
      onClick={onShow}
      title={`Show ${GROUP_LABELS[group]}`}
      aria-label={`Show ${GROUP_LABELS[group]}`}
    >
      <Icon name="dots" />
    </button>
  );
}

export function ProjectPill(props: {
  connection: Connection;
  revision: number;
  health: { passing: number; total: number };
  onHealth: () => void;
  onMenu: () => void;
  onHide: () => void;
}) {
  const { passing, total } = props.health;
  const status = props.connection === "online" ? "SAVED" : props.connection === "connecting" ? "CONNECTING" : "OFFLINE";
  return (
    <div className="project-pill floating">
      <button type="button" className="brand" onClick={props.onMenu} title="Project and workspace" aria-label="Project menu">
        <Icon name="cube" />
        <b>forma</b>
        <Icon name="down" />
      </button>
      <span className="project-name">{projectTitle()}</span>
      <span
        className={`concept ${props.connection}`}
        title={props.connection === "online" ? `Every change is saved before it is confirmed · revision ${props.revision}` : undefined}
      >
        {status}
      </span>
      {total > 0 && (
        <button
          type="button"
          className={`health ${passing === total ? "ok" : "bad"}`}
          onClick={props.onHealth}
          title={passing === total ? "All requirements met" : `${total - passing} of ${total} requirements failing`}
        >
          <Icon name={passing === total ? "shield" : "alert"} />
          {passing === total ? `${passing}/${total}` : `${total - passing} failing`}
        </button>
      )}
      <IconButton icon="close" label="Hide project bar" onClick={props.onHide} />
    </div>
  );
}

export function ToolRail(props: {
  panel: PanelKind | null;
  canEdit: boolean;
  onPanel: (p: PanelKind) => void;
  onSelectTool: () => void;
  onAdd: () => void;
  onStock: () => void;
  stockOpen: boolean;
  onAi: () => void;
  onHide: () => void;
  sketching: boolean;
  onSketch: () => void;
  edging: boolean;
  canEdge: boolean;
  onEdges: () => void;
  faceTool: "shell" | "draft" | null;
  onFaceTool: (kind: "shell" | "draft") => void;
  commenting: boolean;
  /** Open comment threads, shown as a count on the comment tool. */
  openComments: number;
  onComment: () => void;
  /** The bill of materials / sourcing workbench open, if it is. */
  bench: "bom" | "sourcing" | null;
  onBench: (b: "bom" | "sourcing") => void;
  /** The Electronics (PCB) workspace open. */
  pcbOpen: boolean;
  onPcb: () => void;
  /** Drafting on a working plane (lines, arcs, text, dimensions…). */
  drafting?: boolean;
  onDraft?: () => void;
  /** Data: sheets and tables. */
  dataOpen?: boolean;
  onData?: () => void;
  /** The console. */
  consoleOpen?: boolean;
  onConsole?: () => void;
}) {
  const active = (p: PanelKind) => (props.panel === p ? "active" : "");
  return (
    <nav className="tool-rail floating" aria-label="Canvas tools">
      <IconButton icon="dots" label="Project menu" className={active("project")} onClick={() => props.onPanel("project")} />
      <IconButton icon="cursor" label="Select" onClick={props.onSelectTool} />
      <IconButton icon="plus" label="Add box part" onClick={props.onAdd} disabled={!props.canEdit} />
      <IconButton icon="package" label="Stock parts: screws, bearings, motors… (I)" className={props.stockOpen ? "active" : ""} onClick={props.onStock} disabled={!props.canEdit} />
      <IconButton icon="pencil" label="Sketch (K)" className={props.sketching ? "active" : ""} onClick={props.onSketch} disabled={!props.canEdit} />
      {props.onDraft && <IconButton icon="dDim" label="Draft: lines, arcs, splines, text and dimensions in 3D (G)" className={props.drafting ? "active" : ""} onClick={props.onDraft} disabled={!props.canEdit} />}
      <IconButton
        icon="fillet"
        label={props.canEdge || props.edging ? "Round or bevel edges (E)" : "Round or bevel edges (E) · select a part first"}
        className={props.edging ? "active" : ""}
        onClick={props.onEdges}
        disabled={!props.canEdit || !(props.canEdge || props.edging)}
      />
      {(["shell", "draft"] as const).map((k) => {
        const name = k === "shell" ? "Shell: hollow to a wall (H)" : "Draft: taper walls for a mould (D)";
        return (
          <IconButton
            key={k}
            icon={k}
            label={props.canEdge || props.faceTool === k ? name : `${name} · select a part first`}
            className={props.faceTool === k ? "active" : ""}
            onClick={() => props.onFaceTool(k)}
            disabled={!props.canEdit || !(props.canEdge || props.faceTool === k)}
          />
        );
      })}
      <span className="rail-badge-host">
        <IconButton icon="comment" label="Comment (C)" className={props.commenting ? "active" : ""} onClick={props.onComment} />
        {props.openComments > 0 && <span className="rail-badge">{props.openComments > 99 ? "99+" : props.openComments}</span>}
      </span>
      <IconButton icon="layers" label="Project objects" className={active("objects")} onClick={() => props.onPanel("objects")} />
      <IconButton icon="sliders" label="Object properties" className={active("properties")} onClick={() => props.onPanel("properties")} />
      <IconButton icon="shield" label="Requirements" className={active("requirements")} onClick={() => props.onPanel("requirements")} />
      <IconButton icon="history" label="Change history" className={active("history")} onClick={() => props.onPanel("history")} />
      <IconButton icon="chip" label="Electronics: PCB boards (L)" className={props.pcbOpen ? "active" : ""} onClick={props.onPcb} />
      <IconButton icon="list" label="Bill of materials (B)" className={props.bench === "bom" ? "active" : ""} onClick={() => props.onBench("bom")} />
      <IconButton icon="truck" label="Sourcing: suppliers, quotes" className={props.bench === "sourcing" ? "active" : ""} onClick={() => props.onBench("sourcing")} />
      {props.onData && <IconButton icon="table" label="Data: sheets and tables (T)" className={props.dataOpen ? "active" : ""} onClick={props.onData} />}
      {props.onConsole && <IconButton icon="terminal" label="Console (`)" className={props.consoleOpen ? "active" : ""} onClick={props.onConsole} />}
      <IconButton icon="spark" label="Open AI chat (A)" className="ai-tool" onClick={props.onAi} />
      <IconButton icon="minus" label="Hide tools" onClick={props.onHide} />
    </nav>
  );
}

const VIEW_LABELS: Record<ViewName, string> = { perspective: "Perspective", iso: "Isometric", front: "Front", back: "Back", left: "Left", right: "Right", top: "Top", bottom: "Bottom" };

export function ViewBar(props: {
  view: ViewName;
  onViews: () => void;
  onUndo: () => void;
  onZoom: (factor: number) => void;
  onFocus: () => void;
  onStorage: () => void;
  onExport: () => void;
  onHide: () => void;
}) {
  return (
    <div className="view-bar floating" role="toolbar" aria-label="View">
      <button type="button" className="view-select" onClick={props.onViews}>
        {VIEW_LABELS[props.view]} <Icon name="down" />
      </button>
      <i />
      <IconButton icon="undo" label="Undo last change (⌘Z)" onClick={props.onUndo} />
      <IconButton icon="minus" label="Zoom out" onClick={() => props.onZoom(1.25)} />
      <span className="zoom-label">View</span>
      <IconButton icon="plus" label="Zoom in" onClick={() => props.onZoom(0.8)} />
      <i />
      <IconButton icon="expand" label="Hide entire interface (F)" onClick={props.onFocus} />
      <IconButton icon="cloud" label="Project storage" onClick={props.onStorage} />
      <IconButton icon="download" label="Export viewport image" onClick={props.onExport} />
      <IconButton icon="close" label="Hide view bar" onClick={props.onHide} />
    </div>
  );
}

export function Restore({ onRestore }: { onRestore: () => void }) {
  return (
    <button type="button" className="restore floating" onClick={onRestore} title="Restore interface (Escape)" aria-label="Restore interface">
      <Icon name="eye" />
    </button>
  );
}
