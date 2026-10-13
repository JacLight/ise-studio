// Docked panels: flush to a window edge. Panels docked to the same side stack
// in one full-height column (like an IDE's browser over its properties): the
// column has one width, set by the splitter on its inner edge; a splitter
// between two panels shares the height between them; a panel's header
// collapses it (chevron or double-click) and drags it to another place in its
// column or to the other one. The bottom dock spans the view between the
// columns. The 3D view takes what is left. Sizes live in CSS variables on the
// app (`--dock-col-<side>`, `--dockh-<id>`, `--dock-<id>` at the bottom), so a
// splitter drag moves everything without re-rendering.

import { type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { IconButton } from "./icons";
import type { DockSide } from "./ui";

type Column = "left" | "right";

/**
 * One docked panel: its id, edge, the size wanted across (a column's width is
 * its own; for the bottom, the height) and limits (px); in a column, its share
 * of the height (weight), whether it is folded to its header, and the columns
 * it may be dragged to.
 */
export type DockSlot = { id: string; side: DockSide; size: number; min: number; max: number; initial: number; weight?: number; collapsed?: boolean; sides?: Column[] };

/** What a stacked panel's header does: fold it, and drag it elsewhere. */
export type DockStack = { collapsed: boolean; toggle: () => void; grab: (e: PointerEvent<HTMLElement>) => void; dragging: boolean; first: boolean };

/** Where a docked panel (or a column, `col-<side>`) sits, and how far its splitter may go; `top` and `height` in px. */
export type DockPlace = {
  side: DockSide;
  style: CSSProperties;
  size: number;
  min: number;
  max: number;
  initial: number;
  id: string;
  top: number;
  height: number;
  collapsed: boolean;
  first: boolean;
  stack?: DockStack;
};

/** A splitter between two expanded panels of a column. */
export type DockEdge = { above: DockPlace; below: DockPlace };

/** The narrowest (and lowest: a share of the window) the 3D view gets while panels are docked. */
const VIEW_MIN_W = 440;
const viewMinH = (h: number) => Math.max(280, Math.round(h * 0.5));
/** Below this width nothing docks: panels float as on a phone. */
export const DOCK_MIN_WINDOW = 760;
/** A folded panel: its header only. */
export const DOCK_HEAD = 42;
/** The least height a panel open in a column keeps. */
const STACK_MIN = 120;

const v = (name: string) => `var(--${name})`;
const sum = (names: string[]) => (names.length === 0 ? "0px" : names.length === 1 ? v(names[0]) : `calc(${names.map(v).join(" + ")})`);

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

/** Heights of a column's panels: folded ones their header, the rest share what is left by weight (never below STACK_MIN while it fits). */
function stackHeights(members: DockSlot[], h: number): number[] {
  const open = members.filter((m) => !m.collapsed);
  const given = open.map((m) => m.weight).filter((x): x is number => typeof x === "number" && x > 0);
  const usual = given.length ? given.reduce((a, b) => a + b, 0) / given.length : 1;
  const weight = (m: DockSlot) => (m.weight && m.weight > 0 ? m.weight : usual);
  const share = Math.max(0, h - DOCK_HEAD * (members.length - open.length));
  const out = new Map<string, number>();
  let free = open;
  let room = share;
  // Any under the least height take it; the rest share again.
  for (let pass = 0; pass < open.length; pass++) {
    const total = free.reduce((n, m) => n + weight(m), 0);
    const small = free.filter((m) => (room * weight(m)) / total < STACK_MIN);
    if (!small.length || STACK_MIN * open.length > share) {
      for (const m of free) out.set(m.id, (room * weight(m)) / total);
      break;
    }
    for (const m of small) {
      out.set(m.id, STACK_MIN);
      room -= STACK_MIN;
    }
    free = free.filter((m) => !small.includes(m));
    if (!free.length) break;
  }
  // Whole pixels; the last open panel takes what rounding leaves.
  const heights = members.map((m) => (m.collapsed ? DOCK_HEAD : Math.floor(out.get(m.id) ?? STACK_MIN)));
  const last = members.map((m) => !m.collapsed).lastIndexOf(true);
  if (last >= 0) heights[last] += h - heights.reduce((a, b) => a + b, 0);
  return heights;
}

/**
 * Lays docked panels out: a column at each side holding that side's panels
 * one over another, the bottom one between the columns. Column widths shrink
 * (never below their widest panel's minimum) so the view keeps at least
 * VIEW_MIN_W wide, and the bottom keeps the view a good share of the height
 * (viewMinH). Returns each panel's place (and each column's, `col-left` and
 * `col-right`), the splitters between stacked panels, and the CSS variables
 * the app carries (`--dock-l`, `--dock-r`, `--dock-b` are the view's insets).
 */
export function layoutDocks(slots: DockSlot[], w: number, h: number, columns: Record<Column, { size: number; initial: number }>) {
  const bottom = slots.filter((s) => s.side === "bottom");
  const cols = (["left", "right"] as const)
    .map((side) => {
      const members = slots.filter((s) => s.side === side);
      if (!members.length) return null;
      const min = Math.max(...members.map((m) => m.min));
      const max = Math.max(min, Math.max(...members.map((m) => m.max)));
      return { side, members, min, max, size: Math.max(min, Math.min(max, columns[side].size)), initial: columns[side].initial };
    })
    .filter((c): c is NonNullable<typeof c> => !!c);

  // Too wide for the window: give back what each column has above its minimum, in proportion.
  const total = cols.reduce((n, c) => n + c.size, 0);
  const room = Math.max(0, w - VIEW_MIN_W);
  if (total > room) {
    const spare = cols.reduce((n, c) => n + c.size - c.min, 0);
    const cut = Math.min(spare, total - room);
    for (const c of cols) c.size = Math.round(c.size - (spare ? ((c.size - c.min) / spare) * cut : 0));
  }
  const across = cols.reduce((n, c) => n + c.size, 0);
  const size = new Map<string, number>();
  const tall = Math.max(0, h - viewMinH(h));
  for (const s of bottom) size.set(s.id, Math.max(s.min, Math.min(s.max, s.size, tall / bottom.length)));

  const places = new Map<string, DockPlace>();
  const edges: DockEdge[] = [];
  const stacks: Record<Column, DockPlace[]> = { left: [], right: [] };
  const vars: Record<string, string> = {};
  const edge = (side: Column) => (side === "left" ? { left: 0, right: "auto" } : { left: "auto", right: 0 });
  for (const c of cols) {
    const width = v(`dock-col-${c.side}`);
    vars[`--dock-col-${c.side}`] = `${c.size}px`;
    places.set(`col-${c.side}`, {
      id: `col-${c.side}`,
      side: c.side,
      style: { ...edge(c.side), top: 0, bottom: 0, width, height: "auto" },
      size: c.size,
      min: c.min,
      max: Math.max(c.min, Math.min(c.max, w - VIEW_MIN_W - (across - c.size))),
      initial: c.initial,
      top: 0,
      height: h,
      collapsed: false,
      first: true,
    });
    const heights = stackHeights(c.members, h);
    let top = 0;
    c.members.forEach((m, i) => {
      vars[`--dockh-${m.id}`] = `${heights[i]}px`;
      const place: DockPlace = {
        id: m.id,
        side: c.side,
        style: { ...edge(c.side), top: sum(c.members.slice(0, i).map((x) => `dockh-${x.id}`)), bottom: "auto", width, height: v(`dockh-${m.id}`) },
        size: c.size,
        min: c.min,
        max: c.max,
        initial: m.initial,
        top,
        height: heights[i],
        collapsed: !!m.collapsed,
        first: i === 0,
      };
      top += heights[i];
      places.set(m.id, place);
      stacks[c.side].push(place);
    });
    // A splitter at the top of each open panel that has an open one somewhere above.
    const list = stacks[c.side];
    list.forEach((below, i) => {
      if (below.collapsed) return;
      const above = list.slice(0, i).reverse().find((p) => !p.collapsed);
      if (above) edges.push({ above, below });
    });
  }

  const widthOf = (side: Column) => (cols.some((c) => c.side === side) ? v(`dock-col-${side}`) : "0px");
  const bottomIds = bottom.map((s) => `dock-${s.id}`);
  bottom.forEach((s, i) => {
    const others = bottom.reduce((n, x) => n + size.get(x.id)!, 0) - size.get(s.id)!;
    places.set(s.id, {
      id: s.id,
      side: "bottom",
      style: { left: widthOf("left"), right: widthOf("right"), top: "auto", bottom: sum(bottomIds.slice(0, i)), width: "auto", height: v(`dock-${s.id}`) },
      size: size.get(s.id)!,
      min: s.min,
      max: Math.max(s.min, Math.min(s.max, h - viewMinH(h) - others)),
      initial: s.initial,
      top: h - size.get(s.id)!,
      height: size.get(s.id)!,
      collapsed: false,
      first: true,
    });
  });

  vars["--dock-l"] = widthOf("left");
  vars["--dock-r"] = widthOf("right");
  vars["--dock-b"] = sum(bottomIds);
  for (const [id, n] of size) vars[`--dock-${id}`] = `${n}px`;
  return { places, edges, stacks, vars: vars as CSSProperties, viewWidth: w - across };
}

/**
 * The bar on a docked column's (or the bottom panel's) inner edge: drag it to
 * resize (the view follows live), double-click for the usual size, or focus
 * it and use the arrow keys. `onSize` gets the size let go at.
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

/**
 * The bar between two panels stacked in a column (at the top of the lower
 * one): drag it to share their height (everything below follows live),
 * double-click to share it evenly, or focus it and use the arrow keys.
 * `onSize` gets both heights let go at.
 */
export function StackSplitter({ edge, label, onSize }: { edge: DockEdge; label: string; onSize: (above: number, below: number) => void }) {
  const { above, below } = edge;
  const drag = useRef<{ at: number; app: HTMLElement; now: number } | null>(null);
  const [active, setActive] = useState(false);
  const pair = above.height + below.height;
  // How far the lower panel's top may go: each keeps its least height (or what it has, if less).
  const lo = Math.min(above.height, STACK_MIN);
  const hi = pair - Math.min(below.height, STACK_MIN);
  const clamp = (n: number) => Math.round(Math.max(lo, Math.min(hi, n)));
  const set = (app: HTMLElement, a: number) => {
    app.style.setProperty(`--dockh-${above.id}`, `${a}px`);
    app.style.setProperty(`--dockh-${below.id}`, `${pair - a}px`);
  };
  const end = () => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    setActive(false);
    d.app.classList.remove("is-resizing");
    onSize(d.now, pair - d.now);
  };
  return (
    <div
      className={`dock-splitter stack${active ? " active" : ""}`}
      role="separator"
      aria-orientation="horizontal"
      aria-label={label}
      aria-valuenow={above.height}
      aria-valuemin={lo}
      aria-valuemax={hi}
      tabIndex={0}
      title={`${label}: drag to share the height · double-click to share it evenly`}
      onPointerDown={(e: PointerEvent<HTMLDivElement>) => {
        if (e.button > 0) return;
        const app = e.currentTarget.closest(".app") as HTMLElement | null;
        if (!app) return;
        e.preventDefault();
        e.stopPropagation();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { at: e.clientY, app, now: above.height };
        app.classList.add("is-resizing");
        setActive(true);
      }}
      onPointerMove={(e: PointerEvent<HTMLDivElement>) => {
        const d = drag.current;
        if (!d) return;
        d.now = clamp(above.height + e.clientY - d.at);
        set(d.app, d.now);
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={() => onSize(Math.round(pair / 2), pair - Math.round(pair / 2))}
      onKeyDown={(e: KeyboardEvent<HTMLDivElement>) => {
        const move = { ArrowUp: -1, ArrowDown: 1 }[e.key];
        if (!move) return;
        e.preventDefault();
        const a = clamp(above.height + move * (e.shiftKey ? 64 : 16));
        onSize(a, pair - a);
      }}
    />
  );
}

/** The chevron that folds a stacked panel to its header, or opens it again. */
export function CollapseButton({ stack, name }: { stack: DockStack; name: string }) {
  return <IconButton icon="chevron" className={`dock-fold ${stack.collapsed ? "" : "open"}`} pressed={!stack.collapsed} label={stack.collapsed ? `Expand the ${name} panel` : `Collapse the ${name} panel to its header`} onClick={stack.toggle} />;
}

/** Classes for a docked panel's state: folded, being dragged, under another in its column. */
export const dockState = (place: DockPlace | null | undefined) =>
  !place?.stack ? "" : `${place.collapsed ? " is-collapsed" : ""}${place.stack.dragging ? " is-dragging" : ""}${place.first ? "" : " is-stacked"}`;

/** A stacked panel's header: drag it to move the panel, double-click it to fold or open it. */
export function stackHeader(stack: DockStack | undefined, name: string) {
  if (!stack) return {};
  const onControl = (t: EventTarget) => t instanceof Element && !!t.closest("button, input, select, textarea, a, [role=tab], [role=separator]");
  return {
    onPointerDown: (e: PointerEvent<HTMLElement>) => !onControl(e.target) && stack.grab(e),
    onDoubleClick: (e: MouseEvent<HTMLElement>) => !onControl(e.target) && stack.toggle(),
    title: `${name}: drag the header to move the panel (to the other side too) · double-click to ${stack.collapsed ? "expand" : "collapse"} it`,
  };
}

/** Where a dragged panel would land: a column, and its place among that column's panels. */
type Drop = { side: Column; index: number };

/**
 * Dragging a stacked panel by its header: over a column it lands among that
 * column's panels (a line shows where), near a bare window edge it starts a
 * column there, elsewhere it stays in its own column at the height of the
 * pointer. `onDrop` gets the panel, its column and its place there.
 */
export function useDockDrag(stacks: Record<Column, DockPlace[]>, w: number, sidesOf: (id: string) => Column[], onDrop: (id: string, side: Column, index: number) => void) {
  const [drag, setDrag] = useState<{ id: string; x: number; y: number; drop: Drop } | null>(null);
  const live = useRef({ stacks, w, sidesOf, onDrop });
  live.current = { stacks, w, sidesOf, onDrop };

  const dropAt = (id: string, x: number, y: number): Drop => {
    const { stacks, w, sidesOf } = live.current;
    const own = (["left", "right"] as const).find((s) => stacks[s].some((p) => p.id === id)) ?? "right";
    const width = (s: Column) => stacks[s][0]?.size ?? 0;
    const over: Column | null = x < Math.max(width("left"), 90) ? "left" : x > w - Math.max(width("right"), 90) ? "right" : null;
    const side = over && sidesOf(id).includes(over) ? over : own;
    const list = stacks[side].filter((p) => p.id !== id);
    const at = list.findIndex((p) => y < p.top + p.height / 2);
    return { side, index: at < 0 ? list.length : at };
  };

  const grab = (id: string) => (e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    const start = { x: e.clientX, y: e.clientY };
    let moving = false;
    let last: Drop | null = null;
    const move = (m: globalThis.PointerEvent) => {
      if (!moving && Math.hypot(m.clientX - start.x, m.clientY - start.y) < 6) return;
      moving = true;
      last = dropAt(id, m.clientX, m.clientY);
      setDrag({ id, x: m.clientX, y: m.clientY, drop: last });
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      if (moving && last) live.current.onDrop(id, last.side, last.index);
      setDrag(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  // Where it would land: a line across the column (or a band along a bare edge), and the panel's name by the pointer.
  let marker: ReactNode = null;
  if (drag) {
    const list = stacks[drag.drop.side].filter((p) => p.id !== drag.id);
    const width = stacks[drag.drop.side][0]?.size;
    const y = list.length === 0 ? null : drag.drop.index < list.length ? list[drag.drop.index].top : list[list.length - 1].top + list[list.length - 1].height;
    const edge = drag.drop.side === "left" ? { left: 0 } : { right: 0 };
    marker = (
      <>
        <div className="dock-drop band" style={{ ...edge, width: width ?? 300 }} aria-hidden />
        {width && y !== null && <div className="dock-drop" style={{ ...edge, width, top: Math.max(0, Math.min(innerHeight - 4, y - 2)) }} aria-hidden />}
      </>
    );
  }
  return { grab, dragging: drag?.id ?? null, at: drag ? { x: drag.x, y: drag.y } : null, marker };
}
