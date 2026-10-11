// The assistant: one conversation shown as mini, floating, docked, expanded or
// hidden. Changing presentation never clears the draft, messages or proposal.

import { useEffect, useRef } from "react";
import type { Proposal } from "./assistant";
import { useDraggable } from "./drag";
import { Markdown } from "./markdown";
import { Icon, IconButton } from "./icons";
import type { AiMode, Message, Point } from "./ui";

const LOCAL_SUGGESTIONS = ["Add a steel box 60 x 40 x 20", "Make it 80 mm wide", "Use aluminium", "How heavy is it?"];
const CLAUDE_SUGGESTIONS = ["What's in this project?", "Check everything and fix what fails", "Make the selected part 20% lighter", "Design a bracket that holds this"];

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
  onSubmit: (text?: string) => void;
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
}) {
  const ref = useRef<HTMLElement>(null);
  const conversation = useRef<HTMLDivElement>(null);
  const { mode } = props;
  const handle = useDraggable(ref, mode === "floating" ? props.position : null, props.onMoved);

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
  return (
    <section ref={ref} className={`ai-panel floating ai-${mode}`} aria-label="AI assistant">
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
            <button type="button" className={`voice-toggle ${props.voice ? "active" : ""}`} onClick={props.onVoice}>
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
                <button type="button" className="secondary" onClick={props.onVoice}>
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
                    <button type="button" key={s} onClick={() => props.onSubmit(s)}>
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
                  <button type="button" className="primary" onClick={props.onApply} disabled={props.busy}>
                    <Icon name="check" /> Apply change
                  </button>
                  <button type="button" className="secondary" onClick={props.onDiscard} disabled={props.busy}>
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
          props.onSubmit();
        }}
      >
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
              props.onSubmit();
            }
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
          <IconButton icon="mic" label="Preview voice mode" onClick={props.onVoice} />
          {working ? (
            <button type="button" className="send stop" title="Stop" aria-label="Stop Forma AI" onClick={props.onStop}>
              <span className="stop-square" aria-hidden />
            </button>
          ) : (
            <button type="submit" className="send" title="Send message" aria-label="Send message" disabled={props.busy && !props.claude}>
              <Icon name="send" />
            </button>
          )}
        </div>
      </form>
      {!full && <div className="mini-foot">{props.claude ? (working ? "Forma AI is working…" : "Claude on this computer") : "Local command interpreter · no AI model yet"}</div>}
    </section>
  );
}
