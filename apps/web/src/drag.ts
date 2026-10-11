import { type KeyboardEvent, type PointerEvent, type RefObject, useCallback, useEffect, useRef } from "react";
import type { Point } from "./ui";

/** Clamps a panel inside the window and applies its position. */
export function place(el: HTMLElement, p: Point) {
  el.style.left = `${Math.max(8, Math.min(p.x, innerWidth - el.offsetWidth - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(p.y, innerHeight - el.offsetHeight - 8))}px`;
  el.style.right = "auto";
  el.style.bottom = "auto";
}

/** Move a floating panel by its header (pointer or arrow keys). */
export function useDraggable(ref: RefObject<HTMLElement | null>, position: Point | null, onEnd?: (p: Point) => void) {
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el || !position) return;
    place(el, position);
    const onResize = () => place(el, position);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [ref, position]);

  const finish = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    onEnd?.({ x: r.left, y: r.top });
  }, [ref, onEnd]);

  return {
    onPointerDown(e: PointerEvent<HTMLElement>) {
      if ((e.target as HTMLElement).closest("button") || e.button > 0 || !ref.current) return;
      const r = ref.current.getBoundingClientRect();
      drag.current = { x: e.clientX, y: e.clientY, left: r.left, top: r.top };
      e.currentTarget.setPointerCapture(e.pointerId);
    },
    onPointerMove(e: PointerEvent<HTMLElement>) {
      const d = drag.current;
      if (!d || !ref.current) return;
      place(ref.current, { x: d.left + e.clientX - d.x, y: d.top + e.clientY - d.y });
    },
    onPointerUp() {
      if (!drag.current) return;
      drag.current = null;
      finish();
    },
    onPointerCancel() {
      drag.current = null;
    },
    onKeyDown(e: KeyboardEvent<HTMLElement>) {
      const step = { ArrowLeft: [-20, 0], ArrowRight: [20, 0], ArrowUp: [0, -20], ArrowDown: [0, 20] }[e.key];
      if (e.target !== e.currentTarget || !step || !ref.current) return;
      e.preventDefault();
      const r = ref.current.getBoundingClientRect();
      place(ref.current, { x: r.left + step[0], y: r.top + step[1] });
      finish();
    },
  };
}
