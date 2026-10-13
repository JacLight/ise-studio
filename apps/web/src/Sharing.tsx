// View-only links: anyone with the link sees the model in a browser (3D,
// parts, comments), served by this computer's engine on its network address
// (or its internet address, if set). Nobody can change anything through one;
// each link can expire and can be turned off at any time.

import { useCallback, useEffect, useState } from "react";
import { type EntityView, projectUrl } from "./engine";
import { Icon } from "./icons";

type Link = { id: string; label: string; created: number; expires: number | null; expired: boolean; version: string | null; version_name: string | null; urls: string[] };

const EXPIRY: [string, number | null][] = [
  ["Never", null],
  ["1 day", 1],
  ["7 days", 7],
  ["30 days", 30],
];

function until(ms: number) {
  const d = Math.round((ms - Date.now()) / 86400000);
  if (d <= 0) return "expires today";
  return d === 1 ? "expires tomorrow" : `expires in ${d} days`;
}

export function ViewLinks(props: { entities: ReadonlyMap<string, EntityView>; hasPublicUrl: boolean }) {
  const [links, setLinks] = useState<Link[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [label, setLabel] = useState("");
  const [days, setDays] = useState<number | null>(null);
  const [version, setVersion] = useState("");
  const [copied, setCopied] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [turningOff, setTurningOff] = useState<string | null>(null);
  const versions = [...props.entities.values()]
    .map((e) => ({ id: e.id, v: e.components["core.version"] as { name: string; created: number } | undefined }))
    .filter((x) => x.v)
    .sort((a, b) => b.v!.created - a.v!.created);

  const load = useCallback(async () => {
    try {
      const r = await fetch(projectUrl("/shares"));
      if (r.ok) setLinks(((await r.json()) as { links: Link[] }).links);
    } catch {
      setLinks([]);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const send = async (path: string, body: unknown) => {
    setError(null);
    const r = await fetch(projectUrl(path), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) {
      setError(await r.text());
      return null;
    }
    const out = (await r.json()) as { links: Link[]; created?: string };
    setLinks(out.links);
    return out;
  };
  const create = async (e: React.FormEvent) => {
    e.preventDefault();
    const out = await send("/shares", { label, days, version: version || undefined });
    if (out) {
      setAdding(false);
      setLabel("");
      setVersion("");
      const made = out.links.find((l) => l.id === out.created);
      if (made) copy(made);
    }
  };
  const copy = (l: Link) =>
    navigator.clipboard?.writeText(l.urls[0]!).then(
      () => {
        setCopied(l.id);
        window.setTimeout(() => setCopied((c) => (c === l.id ? null : c)), 2000);
      },
      () => {},
    );

  return (
    <div className="view-links">
      <div className="view-links-head">
        <h3>Anyone with the link can view</h3>
        {!adding && (
          <button title="Make a link anyone can open to view the model in a browser" type="button" className="secondary" onClick={() => setAdding(true)}>
            <Icon name="link" /> New link
          </button>
        )}
      </div>
      <p className="view-links-what">They see the model in a browser (orbit, parts, comments) without Forma, and cannot change anything. Program code, the AI chat and history stay private.</p>
      {adding && (
        <form className="view-link-form" onSubmit={create}>
          <label>
            For <span>optional</span>
            <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Supplier quote" maxLength={80} autoFocus />
          </label>
          {versions.length > 0 && (
            <label>
              Shows
              <select title="Show the project live, or fixed at a named version" value={version} onChange={(e) => setVersion(e.target.value)}>
                <option value="">The project as it is (live)</option>
                {versions.map((x) => (
                  <option key={x.id} value={x.id}>
                    Version “{x.v!.name}” (fixed)
                  </option>
                ))}
              </select>
            </label>
          )}
          <div className="segmented" role="radiogroup" aria-label="Link lasts">
            {EXPIRY.map(([text, d]) => (
              <button title={d === null ? "The link works until you turn it off" : `The link stops working after ${text}`} key={text} type="button" role="radio" aria-checked={days === d} className={days === d ? "active" : ""} onClick={() => setDays(d)}>
                {text}
              </button>
            ))}
          </div>
          <div className="row-buttons">
            <button title="Make the view-only link and copy it to the clipboard" type="submit" className="primary">
              <Icon name="link" /> Create and copy link
            </button>
            <button title="Close without making a link" type="button" className="link-button" onClick={() => setAdding(false)}>
              Cancel
            </button>
          </div>
        </form>
      )}
      {links && links.length > 0 && (
        <ul className="view-link-list">
          {links.map((l) => (
            <li key={l.id} className={l.expired ? "is-expired" : ""}>
              <span className="view-link-icon">
                <Icon name={l.expired ? "alert" : "eye"} />
              </span>
              <span className="view-link-text">
                <b>{l.label || (l.version_name ? `Version “${l.version_name}”` : "View-only link")}</b>
                <small>
                  {l.version ? (l.version_name ? `Version “${l.version_name}”` : "A removed version") : "Live"} · {l.expired ? "expired" : l.expires ? until(l.expires) : "no expiry"}
                </small>
                <input className="view-link-url" readOnly value={l.urls[0]} aria-label="Link (select to copy)" onFocus={(e) => e.currentTarget.select()} />
              </span>
              {turningOff === l.id ? (
                <span className="member-remove-ask">
                  Turn off?
                  <button title="Stop this link working for everyone who has it" type="button" className="danger" onClick={() => void send("/shares/revoke", { id: l.id }).then(() => setTurningOff(null))}>
                    Turn off
                  </button>
                  <button title="Leave the link working" type="button" className="link-button" onClick={() => setTurningOff(null)}>
                    Keep
                  </button>
                </span>
              ) : (
                <span className="view-link-actions">
                  {!l.expired && (
                    <button type="button" className="ib" title="Copy link" aria-label="Copy link" onClick={() => copy(l)}>
                      <Icon name={copied === l.id ? "check" : "copy"} />
                    </button>
                  )}
                  <button type="button" className="ib" title="Turn off this link (anyone viewing is dropped)" aria-label="Turn off link" onClick={() => setTurningOff(l.id)}>
                    <Icon name="trash" />
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {links && links.length > 0 && !props.hasPublicUrl && (
        <p className="demo-note">Links open on this network while this computer runs Forma. For people outside it, set an internet address below.</p>
      )}
      {error && <p className="print-note bad">{error}</p>}
    </div>
  );
}
