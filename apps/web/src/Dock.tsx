// Docked panels: flush to a window edge, full height (or, at the bottom, the
// width between the side docks), each resized by a splitter on its inner edge.
// The 3D view takes what is left. Sizes live in CSS variables on the app
// (`--dock-<id>`), so a splitter drag moves everything without re-rendering.

import { type CSSProperties, type KeyboardEvent, type PointerEvent, useEffect, useRef, useState } from "react";
import type { DockSide } from "./ui";

/** One docked panel: its id (for its size), edge, size wanted and limits (px). */
export type DockSlot = { id: string; side: DockSide; size: number; min: number; max: number; initial: number };

/** Where a docked panel sits, and how far its splitter may go. */
export type DockPlace = { side: DockSide; style: CSSProperties; size: number; min: number; max: number; initial: number; id: string };

/** The narrowest (and lowest: a share of the window) the 3D view gets while panels are docked. */
const VIEW_MIN_W = 440;
const viewMinH = (h: number) => Math.max(280, Math.round(h * 0.5));
/** Below this width nothing docks: panels float as on a phone. */
export const DOCK_MIN_WINDOW = 760;

const v = (id: string) => `var(--dock-${id})`;
const sum = (ids: string[]) => (ids.length === 0 ? "0px" : ids.length === 1 ? v(ids[0]) : `calc(${ids.map(v).join(" + ")})`);

/** The window's size, kept up to date. */
export function useWindowSize() {
  const [size, setSize] = useState(() => ({ w: innerWidth, h: innerHeight }));
  useEffect(() => {
    const on = () => setSize({ w: innerWidth, h: innerHeight });
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return size;
}

/**
 * Lays docked panels out: side panels from the window edge in, the bottom
 * one between them. Sizes shrink (never below their minimum) so the view keeps
 * at least VIEW_MIN_W wide and (by viewMinH) a good share of the height. Returns each panel's place and the CSS
 * variables the app carries (`--dock-l`, `--dock-r`, `--dock-b` are the view's
 * insets).
 */
export function layoutDocks(slots: DockSlot[], w: number, h: number) {
  const sides = (side: DockSide) => slots.filter((s) => s.side === side);
  const left = sides("left");
  const right = sides("right");
  const bottom = sides("bottom");
  const size = new Map(slots.map((s) => [s.id, Math.max(s.min, Math.min(s.max, s.size))]));

  // Too wide for the window: give back what each has above its minimum, in proportion.
  const across = [...left, ...right];
  const total = across.reduce((n, s) => n + size.get(s.id)!, 0);
  const room = Math.max(0, w - VIEW_MIN_W);
  if (total > room) {
    const spare = across.reduce((n, s) => n + size.get(s.id)! - s.min, 0);
    const cut = Math.min(spare, total - room);
    for (const s of across) size.set(s.id, Math.round(size.get(s.id)! - (spare ? ((size.get(s.id)! - s.min) / spare) * cut : 0)));
  }
  const tall = Math.max(0, h - viewMinH(h));
  for (const s of bottom) size.set(s.id, Math.max(s.min, Math.min(size.get(s.id)!, tall / bottom.length)));

  const places = new Map<string, DockPlace>();
  const widthOf = (ids: DockSlot[]) => ids.reduce((n, s) => n + size.get(s.id)!, 0);
  const place = (s: DockSlot, style: CSSProperties, max: number) =>
    places.set(s.id, { id: s.id, side: s.side, style, size: size.get(s.id)!, min: s.min, max: Math.max(s.min, Math.min(s.max, max)), initial: s.initial });
  left.forEach((s, i) => {
    const others = widthOf(across) - size.get(s.id)!;
    place(s, { left: sum(left.slice(0, i).map((x) => x.id)), right: "auto", top: 0, bottom: 0, width: v(s.id), height: "auto" }, w - VIEW_MIN_W - others);
  });
  right.forEach((s, i) => {
    const others = widthOf(across) - size.get(s.id)!;
    place(s, { left: "auto", right: sum(right.slice(0, i).map((x) => x.id)), top: 0, bottom: 0, width: v(s.id), height: "auto" }, w - VIEW_MIN_W - others);
  });
  bottom.forEach((s, i) => {
    const others = widthOf(bottom) - size.get(s.id)!;
    place(s, { left: sum(left.map((x) => x.id)), right: sum(right.map((x) => x.id)), top: "auto", bottom: sum(bottom.slice(0, i).map((x) => x.id)), width: "auto", height: v(s.id) }, h - viewMinH(h) - others);
  });

  const vars: Record<string, string> = {
    "--dock-l": sum(left.map((x) => x.id)),
    "--dock-r": sum(right.map((x) => x.id)),
    "--dock-b": sum(bottom.map((x) => x.id)),
  };
  for (const [id, n] of size) vars[`--dock-${id}`] = `${n}px`;
  return { places, vars: vars as CSSProperties, viewWidth: w - widthOf(across) };
}

/**
 * The bar on a docked panel's inner edge: drag it to resize (the view follows
 * live), double-click for the panel's usual size, or focus it and use the
 * arrow keys. `onSize` gets the size let go at.
 */
export function Splitter({ place, label, onSize }: { place: DockPlace; label: string; onSize: (px: number) => void }) {
  const drag = useRef<{ at: number; size: number; app: HTMLElement; now: number } | null>(null);
  const [active, setActive] = useState(false);
  const vertical = place.side !== "bottom";
  // Dragging toward the view grows the panel: right for a left dock, left for a right one, up for the bottom.
  const sign = place.side === "left" ? 1 : -1;
  const clamp = (n: number) => Math.round(Math.max(place.min, Math.min(place.max, n)));
  const set = (app: HTMLElement, n: number) => app.style.setProperty(`--dock-${place.id}`, `${n}px`);

  const end = () => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    setActive(false);
    d.app.classList.remove("is-resizing");
    onSize(d.now);
  };
  return (
    <div
      className={`dock-splitter ${place.side}${active ? " active" : ""}`}
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-label={label}
      aria-valuenow={place.size}
      aria-valuemin={place.min}
      aria-valuemax={place.max}
      tabIndex={0}
      title={`${label}: drag to resize · double-click for the usual size`}
      onPointerDown={(e: PointerEvent<HTMLDivElement>) => {
        if (e.button > 0) return;
        const app = e.currentTarget.closest(".app") as HTMLElement | null;
        if (!app) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { at: vertical ? e.clientX : e.clientY, size: place.size, app, now: place.size };
        app.classList.add("is-resizing");
        setActive(true);
      }}
      onPointerMove={(e: PointerEvent<HTMLDivElement>) => {
        const d = drag.current;
        if (!d) return;
        const delta = (vertical ? e.clientX : e.clientY) - d.at;
        d.now = clamp(d.size + (place.side === "bottom" ? -delta : sign * delta));
        set(d.app, d.now);
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={() => onSize(clamp(place.initial))}
      onKeyDown={(e: KeyboardEvent<HTMLDivElement>) => {
        const grow = vertical ? { ArrowLeft: -sign, ArrowRight: sign }[e.key] : { ArrowUp: 1, ArrowDown: -1 }[e.key];
        if (!grow) return;
        e.preventDefault();
        onSize(clamp(place.size + grow * (e.shiftKey ? 64 : 16)));
      }}
    />
  );
}
