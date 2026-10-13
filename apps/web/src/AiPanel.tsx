// The assistant: one conversation shown as mini, floating, docked, expanded or
// hidden. Changing presentation never clears the draft, messages or proposal.

import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import type { DockPlace } from "./Dock";
import type { Proposal } from "./assistant";
import { type Attachment, attachToAi, attachmentUrl } from "./engine";
import { useDraggable } from "./drag";
import { Markdown } from "./markdown";
import { Icon, IconButton } from "./icons";
import type { AiMode, Message, Point } from "./ui";

const LOCAL_SUGGESTIONS = ["Add a steel box 60 x 40 x 20", "Make it 80 mm wide", "Use aluminium", "How heavy is it?"];
const CLAUDE_SUGGESTIONS = ["What's in this project?", "Check everything and fix what fails", "Make the selected part 20% lighter", "Design a bracket that holds this"];

/** What can be attached: pictures, and PDFs (a drawing, a datasheet). */
const ATTACHABLE = /^(image\/(png|jpeg|gif|webp)|application\/pdf)$/;

/** A file on its way to the conversation: uploading, ready to send, or refused. */
type Pending = { key: number; name: string; preview: string | null; pdf: boolean; attachment?: Attachment; error?: string };

/** Pictures sent with a message: thumbnails (a PDF as a chip), each opening the file. */
function Attachments({ list }: { list: Attachment[] }) {
  return (
    <div className="message-attachments">
      {list.map((a) =>
        a.kind === "pdf" ? (
          <a key={a.id} className="attachment-chip" href={attachmentUrl(a.id)} target="_blank" rel="noreferrer" title={a.name}>
            <span className="attachment-kind">PDF</span>
            {a.name}
          </a>
        ) : (
          <a key={a.id} className="attachment-thumb" href={attachmentUrl(a.id)} target="_blank" rel="noreferrer" title={`${a.name}${a.width ? ` · ${a.width} × ${a.height} px` : ""}`}>
            <img src={attachmentUrl(a.id)} alt={a.name} loading="lazy" />
          </a>
        ),
      )}
    </div>
  );
}

/** What the AI did, in words: "./forma commit shape.update …" → "Changed the design (shape.update)". */
function describeTool(text: string) {
  const forma = text.match(/^\.\/forma\s+(\w+)\s*([\w.]*)/);
  if (forma) {
    const [, cmd, arg] = forma;
    const verbs: Record<string, string> = {
      tree: "Looked over the project",
      list: "Listed everything",
      get: "Read a design in detail",
      check: "Ran the engine's checks",
      ops: "Looked up engine commands",
      run: "Applied a set of changes",
      commit: `Changed the design${arg ? ` · ${arg}` : ""}`,
      view: "Showed it in your view",
      look: "Looked at the model",
      image: arg === "add" ? "Added a picture to the project" : "Listed the project's pictures",
    };
    return verbs[cmd] ?? `Ran ${cmd}`;
  }
  if (/^(Write|Edit)\s/.test(text)) return `Wrote ${text.split("/").pop()}`;
  if (/^Read\s/.test(text)) return `Read ${text.split("/").pop()}`;
  return text.length > 60 ? `${text.slice(0, 57)}…` : text;
}

export function AiPanel(props: {
  mode: AiMode;
  context: string;
  messages: Message[];
  draft: string;
  voice: boolean;
  proposal: Proposal | null;
  busy: boolean;
  position: Point | null;
  onDraft: (text: string) => void;
  onSubmit: (text?: string, attachments?: Attachment[]) => void;
  onMode: (mode: AiMode | "open") => void;
  onVoice: () => void;
  onApply: () => void;
  onDiscard: () => void;
  onMoved: (p: Point) => void;
  /** Talking to a Claude session on this computer (not the local interpreter). */
  claude?: boolean;
  streaming?: string;
  thinking?: boolean;
  aiBusy?: boolean;
  userId?: string;
  onStop?: () => void;
  /** Docked: where it sits (flush to the right edge), with its splitter. */
  dock?: DockPlace | null;
  splitter?: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const conversation = useRef<HTMLDivElement>(null);
  const { mode } = props;
  const handle = useDraggable(ref, mode === "floating" ? props.position : null, props.onMoved);
  const [pending, setPending] = useState<Pending[]>([]);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const nextKey = useRef(1);

  /** Pictures (or PDFs) pasted, dropped or picked: kept by the engine at once, sent with the next message. */
  const attach = useCallback((files: File[]) => {
    for (const file of files) {
      const key = nextKey.current++;
      const pdf = file.type === "application/pdf";
      const name = file.name && file.name !== "image.png" ? file.name : `pasted picture ${key}.${file.type.split("/")[1] ?? "png"}`;
      if (!ATTACHABLE.test(file.type)) {
        setPending((p) => [...p, { key, name, preview: null, pdf, error: "Attach a picture (PNG, JPEG, GIF, WebP) or a PDF" }]);
        continue;
      }
      const preview = pdf ? null : URL.createObjectURL(file);
      setPending((p) => [...p, { key, name, preview, pdf }]);
      attachToAi(file, name).then(
        (attachment) => setPending((p) => p.map((x) => (x.key === key ? { ...x, attachment } : x))),
        (e: Error) => setPending((p) => p.map((x) => (x.key === key ? { ...x, error: e.message } : x))),
      );
    }
  }, []);
  const unattach = (key: number) =>
    setPending((p) => {
      const gone = p.find((x) => x.key === key);
      if (gone?.preview) URL.revokeObjectURL(gone.preview);
      return p.filter((x) => x.key !== key);
    });

  // A picture dropped anywhere on the canvas comes here.
  useEffect(() => {
    const take = (e: Event) => attach([(e as CustomEvent<File>).detail]);
    window.addEventListener("forma:attach", take);
    return () => window.removeEventListener("forma:attach", take);
  }, [attach]);

  const uploading = pending.some((p) => !p.attachment && !p.error);
  const ready = pending.filter((p) => p.attachment).map((p) => p.attachment as Attachment);
  const send = () => {
    if (uploading) return;
    if (!props.draft.trim() && !ready.length) return;
    props.onSubmit(undefined, ready);
    for (const p of pending) if (p.preview) URL.revokeObjectURL(p.preview);
    setPending([]);
  };

  useEffect(() => {
    const c = conversation.current;
    if (c) c.scrollTop = c.scrollHeight;
  }, [props.messages.length, props.proposal, mode, props.streaming]);
  const SUGGESTIONS = props.claude ? CLAUDE_SUGGESTIONS : LOCAL_SUGGESTIONS;
  const working = !!props.claude && !!props.aiBusy;

  if (mode === "hidden") {
    return (
      <button type="button" className="ai-launch floating" onClick={() => props.onMode("open")} aria-label="Open AI chat" title="Open AI chat (A)">
        <Icon name="spark" />
      </button>
    );
  }

  const full = mode !== "mini";
  const docked = mode === "docked" ? (props.dock ?? null) : null;
  return (
    <section
      ref={ref}
      className={`ai-panel ${docked ? `docked dock-${docked.side}` : "floating"} ai-${mode}${dragging ? " dropping" : ""}`}
      style={docked?.style}
      aria-label="AI assistant"
      onDragOver={(e) => {
        if (!props.claude || !e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        e.stopPropagation();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(e) => {
        if (!props.claude || !e.dataTransfer.files.length) return;
        e.preventDefault();
        e.stopPropagation();
        setDragging(false);
        attach([...e.dataTransfer.files]);
      }}
    >
      <header className="panel-handle" tabIndex={mode === "floating" ? 0 : -1} aria-label="Move AI panel using arrow keys" {...(mode === "floating" ? handle : {})}>
        <div className="ai-identity">
          <Icon name="spark" />
          <strong>Forma AI</strong>
          <span className="ready-dot" />
        </div>
        <div className="panel-actions">
          {full ? (
            <IconButton icon="minus" label="Minimize AI" onClick={() => props.onMode("mini")} />
          ) : (
            <IconButton icon="comment" label="Open full conversation" onClick={() => props.onMode("open")} />
          )}
          <IconButton icon="dock" label={mode === "docked" ? "Float AI panel" : "Dock AI panel"} onClick={() => props.onMode(mode === "docked" ? "floating" : "docked")} />
          {full && (
            <IconButton icon="expand" label={mode === "expanded" ? "Restore floating AI" : "Expand AI"} onClick={() => props.onMode(mode === "expanded" ? "floating" : "expanded")} />
          )}
          <IconButton icon="close" label="Hide AI" onClick={() => props.onMode("hidden")} />
        </div>
      </header>

      {full && (
        <>
          <div className="ai-context">
            <span>{props.context}</span>
            <button title={props.voice ? "Back to typing with Forma AI" : "Talk to Forma AI (preview)"} type="button" className={`voice-toggle ${props.voice ? "active" : ""}`} onClick={props.onVoice}>
              <Icon name={props.voice ? "comment" : "mic"} />
              {props.voice ? "Chat" : "Voice"}
            </button>
          </div>
          <div className="conversation" ref={conversation} aria-live="polite">
            {props.voice ? (
              <div className="voice-preview">
                <div className="voice-orb" />
                <h2>Room to think out loud.</h2>
                <p>Voice mode preview</p>
                <span>Microphone off · No audio is recorded</span>
                <button title="Back to typing with Forma AI" type="button" className="secondary" onClick={props.onVoice}>
                  <Icon name="comment" /> Back to chat
                </button>
              </div>
            ) : props.messages.length ? (
              <>
                {props.messages.map((m, i) =>
                  m.role === "tool" ? (
                    <div key={i} className="message tool" title={m.text}>
                      <span className="tool-dot" aria-hidden />
                      {describeTool(m.text)}
                    </div>
                  ) : (
                    <div key={i} className={`message ${m.role}`}>
                      {m.role === "assistant" && props.messages[i - 1]?.role !== "assistant" && (
                        <div className="message-author">
                          <Icon name="spark" /> Forma
                        </div>
                      )}
                      {m.role === "user" && m.user && props.userId && m.user !== props.userId && <div className="message-author other">{m.user}</div>}
                      {m.attachments?.length ? <Attachments list={m.attachments} /> : null}
                      {m.role === "assistant" ? <div className="md"><Markdown text={m.text} /></div> : <p>{m.text}</p>}
                    </div>
                  ),
                )}
                {working && (props.streaming ? (
                  <div className="message assistant streaming">
                    <div className="md">
                      <Markdown text={props.streaming} />
                      <span className="caret" aria-hidden />
                    </div>
                  </div>
                ) : (
                  <div className="message working" aria-label="Forma AI is working">
                    <span className="working-dots" aria-hidden>
                      <i />
                      <i />
                      <i />
                    </span>
                    {props.thinking ? "Thinking" : "Working"}
                  </div>
                ))}
              </>
            ) : (
              <div className="ai-welcome">
                <div className="welcome-symbol">
                  <Icon name="spark" />
                </div>
                <h2>What shall we create?</h2>
                <p>Start with an idea. Shape it here, together.</p>
                <div className="suggestions">
                  {SUGGESTIONS.map((s) => (
                    <button title={`Ask Forma AI: ${s}`} type="button" key={s} onClick={() => props.onSubmit(s)}>
                      {s} <Icon name="chevron" />
                    </button>
                  ))}
                </div>
              </div>
            )}
            {!props.voice && props.proposal && (
              <div className="proposal">
                <span>PROPOSED CHANGE</span>
                <p>{props.proposal.label}</p>
                <div>
                  <button title="Make the proposed change to the model (undoable)" type="button" className="primary" onClick={props.onApply} disabled={props.busy}>
                    <Icon name="check" /> Apply change
                  </button>
                  <button title="Drop the proposed change" type="button" className="secondary" onClick={props.onDiscard} disabled={props.busy}>
                    Discard
                  </button>
                </div>
                <small>Checked by the engine · undoable</small>
              </div>
            )}
          </div>
        </>
      )}

      <form
        className="ai-form"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        {pending.length > 0 && (
          <div className="pending-attachments">
            {pending.map((p) => (
              <div key={p.key} className={`pending-attachment${p.error ? " failed" : ""}${!p.attachment && !p.error ? " uploading" : ""}`} title={p.error ?? p.name}>
                {p.preview ? <img src={p.preview} alt={p.name} /> : <span className="attachment-kind">{p.pdf ? "PDF" : "?"}</span>}
                <button type="button" aria-label={`Remove ${p.name}`} onClick={() => unattach(p.key)}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
            {pending.some((p) => p.error) && <span className="pending-error">{pending.find((p) => p.error)?.error}</span>}
          </div>
        )}
        <label className="sr-only" htmlFor="ai-input">
          Ask Forma AI
        </label>
        <textarea
          id="ai-input"
          rows={full ? 2 : 1}
          placeholder="Describe what you want to create or change…"
          value={props.draft}
          onChange={(e) => props.onDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          onPaste={(e) => {
            if (!props.claude) return;
            const files = [...e.clipboardData.files];
            if (!files.length) return;
            e.preventDefault();
            attach(files);
          }}
        />
        <div className="composer-bottom">
          <span>
            {full
              ? props.claude
                ? "Claude on this computer · every change goes through the engine"
                : "Runs real engine commands · AI model not connected yet"
              : "Your canvas. Your intent."}
          </span>
          {props.claude && (
            <>
              <IconButton icon="paperclip" label="Attach a picture or PDF (or paste, or drop one here)" onClick={() => fileInput.current?.click()} />
              <input
                ref={fileInput}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp,application/pdf"
                multiple
                style={{ display: "none" }}
                onChange={(e) => {
                  attach([...(e.target.files ?? [])]);
                  e.target.value = "";
                }}
              />
            </>
          )}
          <IconButton icon="mic" label="Preview voice mode" onClick={props.onVoice} />
          {working ? (
            <button type="button" className="send stop" title="Stop" aria-label="Stop Forma AI" onClick={props.onStop}>
              <span className="stop-square" aria-hidden />
            </button>
          ) : (
            <button type="submit" className="send" title={uploading ? "Attaching…" : "Send message"} aria-label="Send message" disabled={(props.busy && !props.claude) || uploading}>
              <Icon name="send" />
            </button>
          )}
        </div>
      </form>
      {docked && props.splitter}
      {!full && <div className="mini-foot">{props.claude ? (working ? "Forma AI is working…" : "Claude on this computer") : "Local command interpreter · no AI model yet"}</div>}
    </section>
  );
}
