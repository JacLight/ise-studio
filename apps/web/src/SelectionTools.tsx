// The Views panel's selection section: select all, by type, invert; copy,
// paste, duplicate; hide, isolate and show all. Each also has a shortcut.

import { ITEM_TYPES, type ItemType } from "./selection";

export type SelectionActions = {
  count: number;
  canPaste: boolean;
  onAll: () => void;
  onType: (t: ItemType) => void;
  onInvert: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onDuplicate: () => void;
  onHide: () => void;
  onIsolate: () => void;
  onShowAll: () => void;
};

const mod = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";
const alt = mod === "⌘" ? "⌥" : "Alt+";

export function SelectionTools(a: SelectionActions) {
  const none = a.count === 0;
  return (
    <div className="selection-tools">
      <h4>Selection {a.count > 0 && <span className="muted">· {a.count}</span>}</h4>
      <div className="selection-grid">
        <button type="button" className="vt-btn vt-small" onClick={a.onAll} title={`${mod}A`}>
          All
        </button>
        <button type="button" className="vt-btn vt-small" onClick={a.onInvert} title={`${mod}I`}>
          Invert
        </button>
        <select className="selection-type" value="" aria-label="Select by type" onChange={(e) => e.target.value && a.onType(e.target.value as ItemType)}>
          <option value="">By type…</option>
          {ITEM_TYPES.map((t) => (
            <option key={t.type} value={t.type}>
              {t.label}
            </option>
          ))}
        </select>
      </div>
      <div className="selection-grid">
        <button type="button" className="vt-btn vt-small" onClick={a.onCopy} disabled={none} title={`${mod}C`}>
          Copy
        </button>
        <button type="button" className="vt-btn vt-small" onClick={a.onPaste} disabled={!a.canPaste} title={`${mod}V`}>
          Paste
        </button>
        <button type="button" className="vt-btn vt-small" onClick={a.onDuplicate} disabled={none} title={`${mod}D`}>
          Duplicate
        </button>
      </div>
      <div className="selection-grid">
        <button type="button" className="vt-btn vt-small" onClick={a.onHide} disabled={none} title={`${alt}H`}>
          Hide
        </button>
        <button type="button" className="vt-btn vt-small" onClick={a.onIsolate} disabled={none} title={`${alt}I`}>
          Isolate
        </button>
        <button type="button" className="vt-btn vt-small" onClick={a.onShowAll} title={`${alt}⇧H`}>
          Show all
        </button>
      </div>
      <p className="vt-hint">
        {mod}drag a box: left to right takes what is fully inside, right to left anything it touches (⇧ adds).
      </p>
    </div>
  );
}
