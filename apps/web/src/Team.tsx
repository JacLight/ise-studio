// Team working without a host. Everyone's engine keeps a full copy and syncs
// directly with the others: on the same network by IP, over the internet
// through an address the team chooses. Invites are one per person (a link
// with their own token), shared as a QR code or an email.

import QRCode from "qrcode";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Dialog } from "./Dialog";
import { type Engine, type EntityView, type TeamStatus, projectName, projectTitle, projectUrl } from "./engine";
import { Icon } from "./icons";
import { program } from "./model";
import { ViewLinks } from "./Sharing";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

/** The project's team, refreshed whenever the engine says it changed. */
export function useTeam(engine: Engine) {
  const [team, setTeam] = useState<TeamStatus | null>(null);
  const refresh = useCallback(async () => {
    try {
      const r = await fetch(projectUrl("/team"));
      if (r.ok) setTeam((await r.json()) as TeamStatus);
    } catch {
      /* the engine is restarting: the next event refreshes */
    }
  }, []);
  useEffect(() => {
    void refresh();
    const off = engine.onTeam(() => void refresh());
    const timer = window.setInterval(() => void refresh(), 15000);
    return () => {
      off();
      window.clearInterval(timer);
    };
  }, [engine, refresh]);
  return { team, refresh };
}

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("") || "?";

/** The top-right pill: who is here, and how far changes have travelled. */
export function TeamPill(props: { team: TeamStatus | null; onOpen: () => void; onHide: () => void }) {
  const t = props.team;
  const others = t?.members.filter((m) => !m.me && m.joined) ?? [];
  const online = others.filter((m) => m.online);
  const status = !t?.has_team ? "Saved on this computer" : online.length ? `Delivered to ${online.length} ${online.length === 1 ? "teammate" : "teammates"}` : "Saved locally · teammates offline";
  return (
    <div className="share-pill team-pill floating">
      {t?.has_team && (
        <span className="team-faces" aria-hidden="true">
          {t.members
            .filter((m) => m.joined)
            .slice(0, 4)
            .map((m) => (
              <span key={m.id} className={`face ${m.online ? "on" : ""}`} title={`${m.name}${m.me ? " (you)" : ""} · ${m.online ? "online" : "offline"}`}>
                {initials(m.name)}
              </span>
            ))}
        </span>
      )}
      <span className={`sync-state ${t?.has_team ? (online.length ? "delivered" : "local") : ""}`}>
        <span className="local-dot" />
        {status}
      </span>
      <button title={t?.has_team ? "Team members, invites and connection settings" : "Invite someone to work on this project with you"} type="button" className="share-button" onClick={props.onOpen}>
        <Icon name="share" /> {t?.has_team ? "Team" : "Invite"}
      </button>
      <button type="button" className="ib" title="Hide team controls" aria-label="Hide team controls" onClick={props.onHide}>
        <Icon name="close" />
      </button>
    </div>
  );
}

type Created = { link: string; code: string; addresses: string[]; by: string; role: string; name: string; email: string };

/** Members, invites, connection settings and clashes. */
export function TeamDialog(props: { team: TeamStatus | null; entities: ReadonlyMap<string, EntityView>; run: Run; refresh: () => void; onClose: () => void }) {
  const t = props.team;
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<"editor" | "viewer">("editor");
  const [created, setCreated] = useState<Created | null>(null);
  const [qr, setQr] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [settings, setSettings] = useState(false);
  const [myName, setMyName] = useState(t?.my_name ?? "");
  const [publicUrl, setPublicUrl] = useState(t?.public_url ?? "");
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());
  const [removing, setRemoving] = useState<string | null>(null);
  const remove = async (id: string) => {
    const r = await fetch(projectUrl("/team/remove"), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ member: id }) });
    if (!r.ok) setError(await r.text());
    setRemoving(null);
    props.refresh();
  };

  useEffect(() => {
    if (!created) return setQr(null);
    QRCode.toDataURL(created.link, { margin: 1, width: 360, errorCorrectionLevel: "M", color: { dark: "#2d2840", light: "#ffffff" } }).then(setQr, () => setQr(null));
  }, [created]);

  const invite = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const r = await fetch(projectUrl("/team/invite"), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name, email, role }) });
    if (!r.ok) return setError(await r.text());
    const out = (await r.json()) as Omit<Created, "name" | "email">;
    setCreated({ ...out, name, email });
    setCopied(false);
    props.refresh();
  };

  const saveSettings = async () => {
    const r = await fetch(projectUrl("/team/settings"), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ my_name: myName, public_url: publicUrl }) });
    if (!r.ok) return setError(await r.text());
    setSettings(false);
    props.refresh();
  };

  const mail = created
    ? `mailto:${encodeURIComponent(created.email)}?subject=${encodeURIComponent(`${created.by} invited you to “${projectTitle()}” in Iṣẹ́ Studio`)}&body=${encodeURIComponent(
        `${created.by} invited you to work on “${projectTitle()}” in Iṣẹ́ Studio.\n\nWith Iṣẹ́ Studio running on your computer, open this link:\n\n${created.link}\n\nThe invite is for you alone and works once. Your computer gets its own copy of the project and stays in sync with the team directly.`,
      )}`
    : "";

  // A text message: short, the link on its own line so phones make it tappable.
  const sms = created ? `sms:?&body=${encodeURIComponent(`${created.by} invited you to “${projectTitle()}” in Iṣẹ́ Studio. With Iṣẹ́ Studio running, open:\n${created.link}`)}` : "";

  const where = (addresses: string[]) =>
    addresses.map((a) => {
      const host = a.replace(/^wss?:\/\//, "").replace(/\/peer$/, "");
      return a.startsWith("wss://") || !/^\d+\.\d+\.\d+\.\d+/.test(host) ? `the internet (${host})` : `this network (${host})`;
    });

  const clashes = (t?.conflicts ?? []).filter((c) => !dismissed.has(c.command_id + c.path + c.id));

  return (
    <Dialog title={t?.has_team ? "Team" : "Work together"} onClose={props.onClose}>
      <p>
        Everyone keeps their own full copy of <b>{projectTitle()}</b> and edits it directly, even offline. Copies sync computer to computer: there is no
        server and no host.
      </p>

      {t && t.members.length > 0 && (
        <ul className="team-members">
          {t.members.map((m) => (
            <li key={m.id}>
              <span className={`face ${m.online ? "on" : ""}`}>{initials(m.name)}</span>
              <span className="who">
                <b>
                  {m.name}
                  {m.me ? " (you)" : ""}
                </b>
                <small>{m.email || (m.role === "viewer" ? "Can view" : "Can edit")}</small>
              </span>
              {removing === m.id ? (
                <span className="member-remove-ask">
                  Remove {m.name.split(" ")[0]}?
                  <button title={`Remove ${m.name} from the team: their computer stops syncing`} type="button" className="danger" onClick={() => void remove(m.id)}>
                    Remove
                  </button>
                  <button title={`Keep ${m.name} on the team`} type="button" className="link-button" onClick={() => setRemoving(null)}>
                    Cancel
                  </button>
                </span>
              ) : (
                <>
                  <span className={`member-state ${m.online ? "on" : m.joined ? "" : "invited"}`}>{m.me ? "This computer" : m.online ? "Online" : m.joined ? "Offline" : "Invited"}</span>
                  {!m.me && !t.members.find((x) => x.me && x.role === "viewer") && (
                    <button type="button" className="member-remove" title={`Remove ${m.name} from the team: their computer stops syncing (what they made stays)`} aria-label={`Remove ${m.name}`} onClick={() => setRemoving(m.id)}>
                      <Icon name="close" />
                    </button>
                  )}
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {t && t.removed?.length > 0 && (
        <p className="team-removed">
          Removed: {t.removed.map((r) => `${r.name}${r.by ? ` (by ${r.by})` : ""}`).join(", ")}. Their computers no longer sync with the team.
        </p>
      )}

      {!created ? (
        <form className="invite-form" onSubmit={invite}>
          <h3>Invite someone</h3>
          <div className="invite-row">
            <label>
              Name
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Sam Lee" required />
            </label>
            <label>
              Email <span>optional</span>
              <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="sam@studio.com" />
            </label>
          </div>
          <div className="segmented" role="radiogroup" aria-label="Access">
            {(["editor", "viewer"] as const).map((r) => (
              <button title={r === "editor" ? "They can change the design" : "They can look and comment, not change the design"} key={r} type="button" role="radio" aria-checked={role === r} className={role === r ? "active" : ""} onClick={() => setRole(r)}>
                {r === "editor" ? "Can edit" : "Can view"}
              </button>
            ))}
          </div>
          <button title="Make a one-time invite link for this person" type="submit" className="primary" disabled={!name.trim()}>
            <Icon name="plus" /> Create invite
          </button>
        </form>
      ) : (
        <div className="invite-ready">
          <h3>Invite for {created.name}</h3>
          <div className="invite-share">
            {qr && <img className="invite-qr" src={qr} alt={`QR code of ${created.name}'s invite link`} width={180} height={180} />}
            <div className="invite-actions">
              <p>Scan with their phone to pass the link on, or send it. It opens Iṣẹ́ Studio on their computer and works once, for them alone.</p>
              {created.email ? (
                <a title={`Open your mail app with the invite to ${created.email}`} className="primary invite-mail" href={mail}>
                  <Icon name="mail" /> Email {created.name}
                </a>
              ) : (
                <a title="Open your mail app with the invite link" className="secondary invite-mail" href={mail}>
                  <Icon name="mail" /> Email the invite
                </a>
              )}
              <button title="Copy the invite link to the clipboard"
                type="button"
                className="secondary"
                onClick={() =>
                  navigator.clipboard?.writeText(created.link).then(
                    () => setCopied(true),
                    () => setCopied(false),
                  )
                }
              >
                <Icon name={copied ? "check" : "link"} /> {copied ? "Link copied" : "Copy link"}
              </button>
              <a title="Send the invite link in a text message" className="secondary invite-mail" href={sms}>
                <Icon name="comment" /> Text message
              </a>
            </div>
          </div>
          <input className="invite-link" readOnly value={created.link} aria-label="The invite link (select it to copy)" onFocus={(e) => e.currentTarget.select()} />
          <p className="demo-note">
            Their computer reaches yours through {where(created.addresses).join(" or ")}. {created.addresses.some((a) => a.startsWith("wss://") || !/\/\/\d/.test(a)) ? "" : "Teammates outside this network need an internet address: set one below."}
          </p>
          <button title="Make another invite for someone else" type="button" className="link-button" onClick={() => setCreated(null)}>
            Invite someone else
          </button>
        </div>
      )}

      <ViewLinks entities={props.entities} hasPublicUrl={!!t?.public_url} />

      {clashes.length > 0 && (
        <div className="clashes">
          <h3>Changed at the same time</h3>
          {clashes.slice(0, 6).map((c) => (
            <Clash key={c.command_id + c.path + c.id} clash={c} entities={props.entities} run={props.run} onDone={() => {
                // Hidden at once; the engine forgets it for good (it was kept across restarts until now).
                setDismissed((d) => new Set(d).add(c.command_id + c.path + c.id));
                void fetch(projectUrl("/team/conflicts/settle"), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id: c.id, path: c.path, command_id: c.command_id }) });
              }} />
          ))}
        </div>
      )}

      <div className="team-settings">
        {!settings ? (
          <button title="Set the name teammates see and an internet address for teammates outside this network" type="button" className="link-button" onClick={() => setSettings(true)}>
            <Icon name="sliders" /> Your name and internet address
          </button>
        ) : (
          <>
            <label>
              Your name, as teammates see it
              <input value={myName} onChange={(e) => setMyName(e.target.value)} />
            </label>
            <label>
              Internet address <span>optional</span>
              <input value={publicUrl} onChange={(e) => setPublicUrl(e.target.value)} placeholder="https://ise.yourstudio.com" />
            </label>
            <p>
              For teammates and view-only links outside this network: an address that reaches this computer's Iṣẹ́ Studio (port 7420), through your proxy or an opened port. Connections use HTTPS
              WebSockets, so they pass through proxies. Only teammates' engines and view-only links are answered from outside; the rest stays on this computer.
            </p>
            <button title="Save your name and internet address" type="button" className="secondary" onClick={saveSettings}>
              Save
            </button>
          </>
        )}
      </div>
      {error && <p className="print-note bad">{error}</p>}
    </Dialog>
  );
}

/** One field two teammates (or a branch and its original) changed at once: what each set, which stayed, and a way to pick the other. */
export function Clash(props: { clash: TeamStatus["conflicts"][number]; entities: ReadonlyMap<string, EntityView>; run: Run; onDone: () => void; who?: string; you?: string }) {
  const c = props.clash;
  const e = props.entities.get(c.id);
  const prog = e ? program(e) : undefined;
  const set = e?.components["shape.parameter_set"] as { name: string; params: { key: string; label: string }[] } | undefined;
  const m = /^\/params\/(\d+)\/value$/.exec(c.path);
  const param = m ? (prog?.params ?? set?.params)?.[Number(m[1])] : undefined;
  const thing = e ? (prog?.name ?? set?.name ?? "an item") : "a deleted item";
  const what = param ? `${param.label} of ${thing}` : c.path ? `${thing} (${c.path.replace(/^\//, "").replaceAll("/", " › ")})` : thing;
  const show = (v: unknown) => (typeof v === "number" ? `${Number(v.toFixed(3))}` : typeof v === "string" ? v : JSON.stringify(v));
  const [kept, other] = c.kept === "theirs" ? [c.theirs, c.ours] : [c.ours, c.theirs];
  const pick = async () => {
    if (!param || !e) return;
    const op = prog ? "shape.set_params" : "parameters.set";
    await props.run(op, { target: c.id, values: { [param.key]: other } });
    props.onDone();
  };
  return (
    <div className="clash">
      <p>
        <b>{props.who ?? (c.by.user_id === "local" ? "A teammate" : c.by.user_id)}</b> and {props.you ?? "you"} both changed <b>{what}</b>. Every copy kept <b>{show(kept)}</b>; the other value was{" "}
        <b>{show(other)}</b>.
      </p>
      <div className="clash-actions">
        {param && typeof other === "number" && (
          <button title={`Set ${what} to ${show(other)} instead`} type="button" className="secondary" onClick={pick}>
            Use {show(other)}
          </button>
        )}
        <button title={`Leave ${what} at ${show(kept)} and dismiss`} type="button" className="link-button" onClick={props.onDone}>
          Keep {show(kept)}
        </button>
      </div>
    </div>
  );
}

/** Opened from an invite link: join the project on this computer. */
export function JoinScreen() {
  const code = useMemo(() => new URLSearchParams(location.search).get("invite") ?? "", []);
  const invite = useMemo(() => {
    try {
      const json = atob(code.replace(/-/g, "+").replace(/_/g, "/"));
      return JSON.parse(new TextDecoder().decode(Uint8Array.from(json, (ch) => ch.charCodeAt(0)))) as { p: string; by: string; to: string; a: string[] };
    } catch {
      return null;
    }
  }, [code]);
  const [state, setState] = useState<{ kind: "idle" | "busy" | "error"; text?: string }>({ kind: "idle" });
  const join = async () => {
    setState({ kind: "busy" });
    const r = await fetch("/join", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ invite: code }) });
    if (!r.ok) return setState({ kind: "error", text: await r.text() });
    const out = (await r.json()) as { project: string };
    location.href = `/?project=${encodeURIComponent(out.project)}`;
  };
  const host = invite?.a[0]?.replace(/^wss?:\/\//, "").replace(/\/peer$/, "");
  return (
    <div className="join-screen">
      <div className="join-card">
        <div className="join-brand">
          <Icon name="cube" /> Iṣẹ́ Studio
        </div>
        {invite ? (
          <>
            <h1>
              {invite.by} invited you to <span>“{invite.p.replace(/-/g, " ")}”</span>
            </h1>
            <p>
              You'll join as <b>{invite.to}</b>. This computer gets its own copy of the project, first from {invite.by}'s computer{host ? ` (${host})` : ""}, then
              in sync with the whole team directly. No server in between, and it keeps working offline.
            </p>
            <button title="Copy the project to this computer and join the team" type="button" className="primary" onClick={join} disabled={state.kind === "busy"}>
              {state.kind === "busy" ? "Joining…" : "Join project"}
            </button>
            {state.kind === "error" && <p className="print-note bad">{state.text}</p>}
          </>
        ) : (
          <>
            <h1>That invite link is incomplete</h1>
            <p>Ask whoever invited you to send it again, or to show you its QR code.</p>
          </>
        )}
      </div>
    </div>
  );
}

/** Shown while a newly joined project is still arriving. */
export function SyncBanner(props: { team: TeamStatus | null }) {
  const t = props.team;
  if (!t || !projectName) return null;
  const arriving = t.joining || (t.has_team && t.changes === 0);
  if (!arriving) return null;
  const from = t.online[0]?.name;
  return (
    <div className="sync-banner floating" role="status">
      <span className="spinner" /> {from ? `Getting the project from ${from}…` : "Looking for a teammate's computer to copy the project from…"}
    </div>
  );
}
