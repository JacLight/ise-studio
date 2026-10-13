// Preferences (per viewer): the unit lengths show and are typed in, and
// decimals. Plus the project file: the whole project as one .forma file to
// hand to someone outside the team, and opening one as a new project.

import { useEffect, useRef, useState } from "react";
import { projectName } from "./engine";
import { Icon, IconButton } from "./icons";
import { LENGTH_UNITS, type LengthUnit, setDisplayPrefs, showLength, useDisplayPrefs } from "./units";
import "./data.css";

export function PreferencesDialog(props: { onClose: () => void }) {
  const prefs = useDisplayPrefs();
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} className="prefs-dialog" aria-label="Preferences" onClose={props.onClose} onCancel={(e) => (e.preventDefault(), props.onClose())}>
      <header className="data-head">
        <Icon name="gear" />
        <h2>Preferences</h2>
        <IconButton icon="close" label="Close preferences" onClick={props.onClose} />
      </header>
      <div className="prefs-body">
        <label className="field">
          Lengths in
          <select title="Unit for showing lengths, and for numbers typed without one" value={prefs.length} onChange={(e) => setDisplayPrefs({ length: e.target.value as LengthUnit })}>
            {LENGTH_UNITS.map((u) => (
              <option key={u.unit} value={u.unit}>
                {u.label}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          Decimal places
          <input type="number" min={0} max={6} value={prefs.decimals} onChange={(e) => setDisplayPrefs({ decimals: Math.max(0, Math.min(6, Number(e.target.value) || 0)) })} />
        </label>
        <p className="muted small">
          Fields, dimensions and measurements show lengths this way, and a bare number you type is in this unit (others still work: 25mm, 1in, 0.3m). The model is
          kept in millimetres, so teammates can each choose their own. Example: 25.4 mm shows as <b>{showLength(25.4, prefs)}</b>.
        </p>
        <h3 className="panel-section">Keys</h3>
        <p className="muted small prefs-keys">
          <kbd>`</kbd> console · <kbd>T</kbd> data (sheets, tables) · <kbd>G</kbd> draft on a plane · <kbd>K</kbd> sketch · <kbd>A</kbd> Forma AI · <kbd>F</kbd> focus · <kbd>⌘ ,</kbd> preferences
        </p>
      </div>
    </dialog>
  );
}

/** Rows for the project menu: preferences, console, and the project as one file. */
export function ProjectFileRows(props: { onPrefs: () => void; onConsole: () => void; onData: () => void; notify: (t: string) => void }) {
  const [opening, setOpening] = useState(false);
  const name = projectName || "default";
  return (
    <div className="project-file-rows">
      <button title="Set the length unit and decimal places (⌘ ,)" type="button" className="list-row" onClick={props.onPrefs}>
        <Icon name="gear" />
        <span>Preferences: units</span>
        <kbd>⌘ ,</kbd>
      </button>
      <button title="Open sheets and tables of project data (T)" type="button" className="list-row" onClick={props.onData}>
        <Icon name="table" />
        <span>Data: sheets and tables</span>
        <kbd>T</kbd>
      </button>
      <button title="Open the command console (`)" type="button" className="list-row" onClick={props.onConsole}>
        <Icon name="terminal" />
        <span>Console</span>
        <kbd>`</kbd>
      </button>
      <a className="list-row" href={`/projects/${encodeURIComponent(name)}/bundle`} download title="The whole project, its history and imported files, as one file to send to someone outside the team">
        <Icon name="download" />
        <span>Download project file (.forma)</span>
      </a>
      <label className="list-row file-row" title="Opens a .forma project file as a new project">
        <Icon name="upload" />
        <span>{opening ? "Opening…" : "Open a project file…"}</span>
        <input
          type="file"
          accept=".forma"
          aria-label="Open a project file"
          onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (!f) return;
            setOpening(true);
            try {
              const res = await fetch("/projects/import", { method: "POST", body: f });
              if (!res.ok) throw new Error(await res.text());
              const r = (await res.json()) as { name: string };
              const url = new URL(location.href);
              url.searchParams.set("project", r.name);
              location.assign(url.toString());
            } catch (err) {
              props.notify(`Could not open ${f.name}: ${(err as Error).message}`);
            } finally {
              setOpening(false);
            }
          }}
        />
      </label>
    </div>
  );
}
