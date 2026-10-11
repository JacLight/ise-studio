// Comments pinned to the model. Threads and replies are part of the project
// (saved in its journal, synced to teammates, there offline); pins sit on a
// point of a part in its own coordinates, so they travel with it.

import { useEffect, useMemo, useRef, useState } from "react";
import type { EntityView } from "./engine";
import { Icon } from "./icons";
import { partName } from "./model";

type Vec3 = [number, number, number];

/** A freehand stroke or arrow drawn on the model (points in the pinned part's coordinates). */
export type Stroke = { points: Vec3[]; arrow?: boolean };
export type Anchor = { target?: string | null; part?: string | null; point: Vec3; normal?: Vec3 | null };
export type ThreadData = {
  anchor: Anchor;
  author: string;
  created: number;
  text: string;
  resolved?: boolean;
  resolved_by?: string | null;
  resolved_at?: number | null;
  edited?: number | null;
  markup?: Stroke[];
};
export type ReplyData = { thread: string; author: string; created: number; text: string; edited?: number | null };

export const threadOf = (e: EntityView | undefined) => e?.components["comment.thread"] as ThreadData | undefined;
export const replyOf = (e: EntityView | undefined) => e?.components["comment.reply"] as ReplyData | undefined;

/** One thread as shown: its number (by when it was started), its replies, and where it is. */
export type Thread = ThreadData & { id: string; number: number; replies: (ReplyData & { id: string })[]; on: string | null; gone: boolean };

/** Where a pin is drawn and what it shows. */
export type CommentPin = {
  id: string;
  number: number;
  initials: string;
  color: string;
  target: string | null;
  point: Vec3;
  normal: Vec3 | null;
  resolved: boolean;
  replies: number;
  markup: Stroke[];
  /** Open in a card: drawn raised, its markup at full strength. */
  draft?: boolean;
};

/** A spot clicked on the model for a new pin. */
export type PinAnchor = { target: string; part: string | null; point: Vec3; normal: Vec3 | null };

export type MarkupTool = "pen" | "arrow";

/** What the viewport shows of comments. */
export type CommentsView = {
  pins: CommentPin[];
  /** The thread whose card is open ("draft" for a new one). */
  active: string | null;
  /** Comment mode: a click on the model drops a pin. */
  placing: boolean;
  /** Drawing markup on the new comment's part. */
  tool: MarkupTool | null;
  me: { initials: string; color: string };
  card: React.ReactNode;
  onPlace: (a: PinAnchor) => void;
  onOpen: (id: string | null) => void;
  onStroke: (s: Stroke) => void;
};

/** Avatar colours, by name: the same person is the same colour on every computer. */
const AVATAR_COLORS = ["#e2553f", "#1f9d74", "#2f7de1", "#d4880f", "#c23d8f", "#0f9fb3", "#7352d6", "#5a8f29"];
export const colorFor = (name: string) => AVATAR_COLORS[[...name].reduce((h, ch) => (h * 31 + ch.charCodeAt(0)) >>> 0, 7) % AVATAR_COLORS.length]!;
export const initialsOf = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => [...w][0]!.toUpperCase())
    .join("") || "?";

/** "just now", "5 min ago", "3 h ago", "yesterday", "12 Mar". */
export function ago(ms: number, now = Date.now()) {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  if (s < 172800) return "yesterday";
  const d = new Date(ms);
  return d.toLocaleDateString(undefined, d.getFullYear() === new Date(now).getFullYear() ? { day: "numeric", month: "short" } : { day: "numeric", month: "short", year: "numeric" });
}

/** Every thread, numbered in the order they were started (the same on every copy). */
export function useThreads(entities: ReadonlyMap<string, EntityView>): Thread[] {
  return useMemo(() => {
    const replies = new Map<string, (ReplyData & { id: string })[]>();
    const roots: (ThreadData & { id: string })[] = [];
    for (const e of entities.values()) {
      const t = threadOf(e);
      if (t) roots.push({ ...t, id: e.id });
      const r = replyOf(e);
      if (r) replies.set(r.thread, [...(replies.get(r.thread) ?? []), { ...r, id: e.id }]);
    }
    const order = (a: { created: number; id: string }, b: { created: number; id: string }) => a.created - b.created || a.id.localeCompare(b.id);
    roots.sort(order);
    return roots.map((t, i) => {
      const target = t.anchor.target ? entities.get(t.anchor.target) : undefined;
      const on = target ? [partName(target), t.anchor.part].filter(Boolean).join(" › ") : null;
      return { ...t, number: i + 1, replies: (replies.get(t.id) ?? []).sort(order), on, gone: !!t.anchor.target && !target };
    });
  }, [entities]);
}

/** Pins for the threads shown. */
export function pinsFor(threads: Thread[], showResolved: boolean): CommentPin[] {
  return threads
    .filter((t) => !t.gone && (showResolved || !t.resolved))
    .map((t) => ({
      id: t.id,
      number: t.number,
      initials: initialsOf(t.author),
      color: colorFor(t.author),
      target: t.anchor.target ?? null,
      point: t.anchor.point,
      normal: t.anchor.normal ?? null,
      resolved: !!t.resolved,
      replies: t.replies.length,
      markup: t.markup ?? [],
    }));
}

export function Avatar({ name, size = 26 }: { name: string; size?: number }) {
  return (
    <span className="comment-avatar" style={{ "--who": colorFor(name), width: size, height: size, fontSize: size * 0.4 } as React.CSSProperties} title={name}>
      {initialsOf(name)}
    </span>
  );
}

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

/** A text box that grows with what is typed; Enter sends, Shift+Enter starts a new line. */
function Composer(props: { placeholder: string; submit: string; onSubmit: (text: string) => Promise<boolean>; initial?: string; autoFocus?: boolean; onCancel?: () => void; extra?: React.ReactNode }) {
  const [text, setText] = useState(props.initial ?? "");
  const [sending, setSending] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);
  useEffect(() => {
    if (props.autoFocus) ref.current?.focus({ preventScroll: true });
  }, [props.autoFocus]);
  const send = async () => {
    const t = text.trim();
    if (!t || sending) return;
    setSending(true);
    const ok = await props.onSubmit(t);
    setSending(false);
    if (ok) setText("");
  };
  return (
    <div className="comment-composer">
      <textarea
        ref={ref}
        rows={1}
        value={text}
        maxLength={4000}
        placeholder={props.placeholder}
        aria-label={props.placeholder}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void send();
          } else if (e.key === "Escape" && props.onCancel) {
            e.preventDefault();
            e.stopPropagation();
            props.onCancel();
          }
        }}
      />
      <div className="composer-row">
        {props.extra}
        <span className="composer-gap" />
        {props.onCancel && (
          <button type="button" className="composer-cancel" onClick={props.onCancel}>
            Cancel
          </button>
        )}
        <button type="button" className="composer-send" disabled={!text.trim() || sending} onClick={() => void send()} aria-label={props.submit} title={`${props.submit} (Enter)`}>
          <Icon name="send" />
        </button>
      </div>
    </div>
  );
}

function Message(props: { id: string; author: string; created: number; text: string; edited?: number | null; mine: boolean; run: Run; me: string; root?: boolean; onDeleted?: () => void }) {
  const [editing, setEditing] = useState(false);
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="comment-message">
      <Avatar name={props.author} />
      <div className="comment-body">
        <div className="comment-meta">
          <b>{props.author}</b>
          <time dateTime={new Date(props.created).toISOString()} title={new Date(props.created).toLocaleString()}>
            {ago(props.created)}
            {props.edited ? " · edited" : ""}
          </time>
          {props.mine && !editing && (
            <span className="comment-own">
              <button type="button" className="ib" title="Edit" aria-label="Edit comment" onClick={() => setEditing(true)}>
                <Icon name="pencil" />
              </button>
              <button type="button" className="ib" title={props.root ? "Delete thread" : "Delete reply"} aria-label={props.root ? "Delete thread" : "Delete reply"} onClick={() => setConfirm(true)}>
                <Icon name="trash" />
              </button>
            </span>
          )}
        </div>
        {editing ? (
          <Composer
            placeholder="Edit comment"
            submit="Save"
            initial={props.text}
            autoFocus
            onCancel={() => setEditing(false)}
            onSubmit={async (text) => {
              const ok = (await props.run("comment.edit", { target: props.id, text, author: props.me })) !== null;
              if (ok) setEditing(false);
              return ok;
            }}
          />
        ) : (
          <p className="comment-text">{props.text}</p>
        )}
        {confirm && (
          <div className="comment-confirm" role="alert">
            {props.root ? "Delete this thread and its replies?" : "Delete this reply?"}
            <button type="button" className="danger" onClick={() => void props.run("comment.delete", { target: props.id, author: props.me }).then((r) => r !== null && props.onDeleted?.())}>
              Delete
            </button>
            <button type="button" onClick={() => setConfirm(false)}>
              Keep
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/** An open thread: the comment, its replies, a reply box, resolve. */
export function ThreadCard(props: { thread: Thread; me: string; run: Run; onClose: () => void }) {
  const t = props.thread;
  const end = useRef<HTMLDivElement>(null);
  const count = t.replies.length;
  useEffect(() => end.current?.scrollIntoView({ block: "nearest" }), [count]);
  return (
    <div className={`comment-card ${t.resolved ? "is-resolved" : ""}`} role="dialog" aria-label={`Comment ${t.number}`} onPointerDown={(e) => e.stopPropagation()}>
      <header className="comment-card-head">
        <span className="comment-num" style={{ "--who": colorFor(t.author) } as React.CSSProperties}>
          {t.number}
        </span>
        <span className="comment-where">{t.gone ? "On a deleted part" : (t.on ?? "On the model")}</span>
        <button
          type="button"
          className={`comment-resolve ${t.resolved ? "on" : ""}`}
          title={t.resolved ? `Resolved by ${t.resolved_by ?? "someone"} · reopen` : "Resolve"}
          aria-label={t.resolved ? "Reopen thread" : "Resolve thread"}
          aria-pressed={!!t.resolved}
          onClick={() => void props.run(t.resolved ? "comment.reopen" : "comment.resolve", { target: t.id, author: props.me })}
        >
          <Icon name="check" />
        </button>
        <button type="button" className="ib" title="Close (Esc)" aria-label="Close comment" onClick={props.onClose}>
          <Icon name="close" />
        </button>
      </header>
      <div className="comment-thread">
        <Message id={t.id} author={t.author} created={t.created} text={t.text} edited={t.edited} mine={t.author === props.me} run={props.run} me={props.me} root onDeleted={props.onClose} />
        {t.replies.map((r) => (
          <Message key={r.id} id={r.id} author={r.author} created={r.created} text={r.text} edited={r.edited} mine={r.author === props.me} run={props.run} me={props.me} />
        ))}
        {t.resolved && (
          <p className="comment-resolved-note">
            <Icon name="check" /> Resolved by {t.resolved_by ?? "someone"}
            {t.resolved_at ? ` · ${ago(t.resolved_at)}` : ""}
          </p>
        )}
        <div ref={end} />
      </div>
      <Composer
        placeholder={t.resolved ? "Reply (thread is resolved)" : "Reply"}
        submit="Reply"
        onSubmit={async (text) => (await props.run("comment.reply", { thread: t.id, text, author: props.me })) !== null}
      />
    </div>
  );
}

/** A new pin: what to say, and optionally marks drawn on the part. */
export function DraftCard(props: {
  me: string;
  where: string;
  strokes: number;
  tool: MarkupTool | null;
  onTool: (t: MarkupTool | null) => void;
  onUndoStroke: () => void;
  onPost: (text: string) => Promise<boolean>;
  onCancel: () => void;
}) {
  return (
    <div className="comment-card is-draft" role="dialog" aria-label="New comment" onPointerDown={(e) => e.stopPropagation()}>
      <header className="comment-card-head">
        <Avatar name={props.me} size={24} />
        <span className="comment-where">{props.where}</span>
        <button type="button" className="ib" title="Cancel (Esc)" aria-label="Cancel comment" onClick={props.onCancel}>
          <Icon name="close" />
        </button>
      </header>
      {props.tool && <p className="markup-hint">Drag on the model to draw {props.tool === "arrow" ? "an arrow" : "a mark"} · click the tool again to orbit</p>}
      <Composer
        placeholder="Add a comment"
        submit="Post"
        autoFocus
        onCancel={props.onCancel}
        onSubmit={props.onPost}
        extra={
          <span className="markup-tools" role="group" aria-label="Markup">
            <button type="button" className={`ib ${props.tool === "pen" ? "active" : ""}`} aria-pressed={props.tool === "pen"} title="Draw on the model" aria-label="Draw on the model" onClick={() => props.onTool(props.tool === "pen" ? null : "pen")}>
              <Icon name="pencil" />
            </button>
            <button type="button" className={`ib ${props.tool === "arrow" ? "active" : ""}`} aria-pressed={props.tool === "arrow"} title="Draw an arrow" aria-label="Draw an arrow" onClick={() => props.onTool(props.tool === "arrow" ? null : "arrow")}>
              <Icon name="arrow" />
            </button>
            {props.strokes > 0 && (
              <button type="button" className="ib" title="Undo last mark" aria-label="Undo last mark" onClick={props.onUndoStroke}>
                <Icon name="undo" />
              </button>
            )}
          </span>
        }
      />
    </div>
  );
}

/** Every thread: open or resolved, newest activity first; a click flies to its pin. */
export function CommentsPanel(props: {
  threads: Thread[];
  active: string | null;
  filter: "open" | "resolved" | "all";
  onFilter: (f: "open" | "resolved" | "all") => void;
  pinsShown: boolean;
  onPinsShown: (on: boolean) => void;
  onOpen: (id: string) => void;
  onNew: () => void;
}) {
  const open = props.threads.filter((t) => !t.resolved).length;
  const resolved = props.threads.length - open;
  const last = (t: Thread) => Math.max(t.created, t.replies.at(-1)?.created ?? 0);
  const shown = props.threads.filter((t) => (props.filter === "all" ? true : props.filter === "open" ? !t.resolved : t.resolved)).sort((a, b) => last(b) - last(a));
  return (
    <div className="comments-panel">
      <div className="seg" role="tablist" aria-label="Which comments">
        {(
          [
            ["open", `Open ${open || ""}`],
            ["resolved", `Resolved ${resolved || ""}`],
            ["all", "All"],
          ] as const
        ).map(([k, label]) => (
          <button key={k} type="button" role="tab" aria-selected={props.filter === k} className={props.filter === k ? "active" : ""} onClick={() => props.onFilter(k)}>
            {label.trim()}
          </button>
        ))}
      </div>
      <div className="comments-tools">
        <button type="button" className="secondary" onClick={props.onNew}>
          <Icon name="comment" /> New comment <kbd>C</kbd>
        </button>
        <label className="comments-toggle" title="Show pins on the model">
          <input type="checkbox" checked={props.pinsShown} onChange={(e) => props.onPinsShown(e.target.checked)} /> Pins
        </label>
      </div>
      {shown.length === 0 ? (
        <div className="comments-empty">
          <span className="comments-empty-pin">
            <Icon name="comment" />
          </span>
          {props.threads.length === 0 ? (
            <p>
              No comments yet. Press <kbd>C</kbd> and click the model to pin one. Teammates see it as soon as you post, and it follows the part when it moves.
            </p>
          ) : props.filter === "open" ? (
            <p>Every thread is resolved.</p>
          ) : (
            <p>Nothing resolved yet.</p>
          )}
        </div>
      ) : (
        <ul className="comment-list">
          {shown.map((t) => (
            <li key={t.id}>
              <button type="button" className={`comment-row ${props.active === t.id ? "active" : ""} ${t.resolved ? "is-resolved" : ""}`} onClick={() => props.onOpen(t.id)}>
                <span className="comment-num" style={{ "--who": colorFor(t.author) } as React.CSSProperties}>
                  {t.resolved ? <Icon name="check" /> : t.number}
                </span>
                <span className="comment-row-body">
                  <span className="comment-meta">
                    <b>{t.author}</b>
                    <time>{ago(last(t))}</time>
                  </span>
                  <span className="comment-preview">{t.text}</span>
                  <span className="comment-row-foot">
                    {t.gone ? "On a deleted part" : (t.on ?? "On the model")}
                    {t.replies.length > 0 && (
                      <span className="comment-replies">
                        {t.replies.length} {t.replies.length === 1 ? "reply" : "replies"}
                      </span>
                    )}
                    {(t.markup?.length ?? 0) > 0 && <span className="comment-replies">markup</span>}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** What a new pin is on, in words. */
export function placeName(a: PinAnchor, entities: ReadonlyMap<string, EntityView>) {
  const e = entities.get(a.target);
  if (!e) return "On the model";
  return [partName(e), a.part].filter(Boolean).join(" › ");
}
