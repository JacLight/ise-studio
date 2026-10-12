// Value field you can drag, type math into, or nudge with arrow keys.
// Drag the label to scrub (pointer-locked, so it never runs out of screen).
// ⇧ = ×10, ⌥ = ×0.1. Typing accepts units and expressions: 2in, +10, *2, 40/3.
// Changes preview live through `onLive` and commit once through `onCommit`.

import { type CSSProperties, useEffect, useRef, useState } from "react";
import { evaluate as evaluateIn } from "./expr";
import { UNIT_MM, useDisplayPrefs } from "./units";

type Props = {
  label: string;
  /** Short chip text, e.g. "W" or "X". */
  chip: string;
  /** Accent for the chip, e.g. CAD axis colors. */
  tone?: string;
  value: number;
  unit?: string;
  min?: number;
  max?: number;
  /** Size of one scrub step in value units. */
  step?: number;
  onLive: (value: number | null) => void;
  onCommit: (value: number) => void;
  /** The formula that drives this value ("=Desk size.width"), if it is linked. */
  formula?: string;
  /** Typing "=…" links the value to a formula (and editing a linked one changes it). */
  onFormula?: (formula: string) => void;
};

const PIXELS_PER_STEP = 4;

const round = (v: number, step: number) => Math.round(v / step) * step;
const display = (v: number, places = 2) => String(Number(v.toFixed(places)));

function modifier(e: { shiftKey: boolean; altKey: boolean }) {
  return e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
}

export function ScrubField({ label, chip, tone, value, unit: given = "mm", min, max, step = 1, onLive, onCommit, formula, onFormula }: Props) {
  // Lengths show (and are typed) in the person's chosen unit; the engine keeps millimetres.
  const prefs = useDisplayPrefs();
  const length = given === "mm";
  const unitMm = length ? UNIT_MM[prefs.length] : 1;
  const unit = length ? prefs.length : given;
  const places = length ? prefs.decimals + (unitMm >= 1000 || unitMm === 304.8 ? 2 : unitMm > 1 ? 1 : 0) : 2;
  const evaluate = (text: string, current: number) => evaluateIn(text, current, unitMm);
  const [draft, setDraft] = useState<string | null>(null);
  const [scrubbing, setScrubbing] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const [shake, setShake] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const pending = useRef<number | null>(null);
  const nudgeTimer = useRef<number>(undefined);
  const live = useRef<number | null>(null);

  const clamp = (v: number) => Math.min(max ?? Infinity, min !== undefined ? Math.max(min, v) : v);

  const preview = (v: number | null) => {
    live.current = v;
    onLive(v);
  };

  const commit = (v: number) => {
    window.clearTimeout(nudgeTimer.current);
    pending.current = null;
    if (Math.abs(v - value) < 1e-9) {
      preview(null);
      return;
    }
    // The committed value arrives through `value`; the field holds no preview of its own after this.
    live.current = null;
    onCommit(v);
  };

  // A new engine value (from anyone) replaces any stale local preview.
  useEffect(() => {
    if (!scrubbing && pending.current === null) live.current = null;
  }, [value, scrubbing]);

  useEffect(() => () => window.clearTimeout(nudgeTimer.current), []);

  const shown = draft ?? display((live.current ?? value) / unitMm, places);

  const startScrub = (e: React.PointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget;
    const start = value;
    let travelled = 0;
    let moved = false;
    setScrubbing(true);
    target.setPointerCapture(e.pointerId);
    target.requestPointerLock?.()?.catch?.(() => undefined);

    const onMove = (ev: PointerEvent) => {
      travelled += ev.movementX;
      if (Math.abs(travelled) > 2) moved = true;
      const s = step * modifier(ev);
      preview(clamp(round(start + (travelled / PIXELS_PER_STEP) * s, Math.min(s, step))));
    };
    const finish = (cancelled: boolean) => {
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
      target.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKey, true);
      if (document.pointerLockElement) document.exitPointerLock();
      setScrubbing(false);
      const final = live.current;
      if (cancelled || !moved || final === null) {
        preview(null);
        if (!moved && !cancelled) input.current?.focus();
      } else {
        commit(final);
      }
    };
    const onUp = () => finish(false);
    const onCancel = () => finish(true);
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") {
        ev.stopPropagation();
        finish(true);
      }
    };
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
    target.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKey, true);
  };

  const isFormula = (text: string) => !!onFormula && text.trim().startsWith("=");
  const submitDraft = () => {
    if (draft === null) return true;
    if (isFormula(draft)) {
      if (draft.trim() !== formula) onFormula!(draft.trim());
      setDraft(null);
      setInvalid(false);
      return true;
    }
    const v = evaluate(draft, value);
    if (v === null || (min !== undefined && v < min) || (max !== undefined && v > max)) {
      setInvalid(true);
      setShake((n) => n + 1);
      return false;
    }
    setDraft(null);
    setInvalid(false);
    commit(clamp(v));
    return true;
  };

  return (
    <div
      className={`scrub ${scrubbing ? "is-scrubbing" : ""} ${invalid ? "is-invalid" : ""} ${formula ? "is-linked" : ""}`}
      title={formula ? `Follows ${formula}` : undefined}
      style={{ "--tone": tone } as CSSProperties}
      data-shake={shake % 2}
    >
      <span
        className="scrub-handle"
        onPointerDown={(e) => {
          // A linked value follows its formula; it is not dragged.
          if (!formula) startScrub(e);
        }}
        title={formula ? `Linked: ${formula}` : `Drag to adjust ${label.toLowerCase()} · ⇧ ×10 · ⌥ ×0.1${onFormula ? " · type = to link" : ""}`}
        aria-hidden
      >
        <span className="scrub-chip">{chip}</span>
      </span>
      <input
        ref={input}
        aria-label={`${label} in ${unit}`}
        inputMode={onFormula ? "text" : "decimal"}
        spellCheck={false}
        value={shown}
        onFocus={(e) => {
          // Editing a linked value edits its formula.
          if (formula) setDraft(formula);
          const el = e.currentTarget;
          requestAnimationFrame(() => el.select());
        }}
        onChange={(e) => {
          setDraft(e.target.value);
          setInvalid(!isFormula(e.target.value) && evaluate(e.target.value, value) === null && e.target.value.trim() !== "");
        }}
        onBlur={() => {
          if (pending.current !== null) commit(pending.current);
          if (!submitDraft()) {
            setDraft(null);
            setInvalid(false);
          }
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            if (submitDraft()) e.currentTarget.blur();
          } else if (e.key === "Escape") {
            e.stopPropagation();
            window.clearTimeout(nudgeTimer.current);
            pending.current = null;
            setDraft(null);
            setInvalid(false);
            preview(null);
            e.currentTarget.blur();
          } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !formula && !(draft && isFormula(draft))) {
            e.preventDefault();
            setDraft(null);
            const s = step * modifier(e);
            const base = pending.current ?? value;
            const next = clamp(round(base + (e.key === "ArrowUp" ? s : -s), Math.min(s, step)));
            pending.current = next;
            preview(next);
            window.clearTimeout(nudgeTimer.current);
            nudgeTimer.current = window.setTimeout(() => pending.current !== null && commit(pending.current), 650);
          }
        }}
      />
      <span className="scrub-unit">{unit}</span>
    </div>
  );
}
