// One tooltip for the whole app: every button, icon, link and control that
// carries a title or an aria-label shows it on hover (and on keyboard focus)
// after a short pause, styled and kept on screen. The native title is lifted
// while the tip shows so the two never double up.

const DELAY = 350;
/** Moving from one control to the next within this long shows the next tip at once. */
const WARM = 600;
const TARGETS = "button, a[href], select, summary, label, input, [role=button], [role=tab], [role=menuitem], [role=menuitemradio], [data-tip], [title]";

let tip: HTMLDivElement | null = null;
let current: HTMLElement | null = null;
let timer = 0;
let lastHidden = 0;

function textOf(el: HTMLElement): string {
  const own = el.dataset.tip || el.getAttribute("title") || el.dataset.tipTitle || el.getAttribute("aria-label") || "";
  if (own.trim()) return own.trim();
  // An unlabeled control inside a labeled one (an icon in a button).
  return "";
}

function targetOf(node: EventTarget | null): HTMLElement | null {
  let el = node instanceof Element ? (node.closest(TARGETS) as HTMLElement | null) : null;
  // Walk up past plain wrappers that have nothing to say.
  while (el && !textOf(el)) el = el.parentElement?.closest(TARGETS) as HTMLElement | null;
  if (!el || el.closest("[data-no-tip]")) return null;
  // Text fields say what they are with their placeholder; no tip while typing.
  if ((el instanceof HTMLInputElement && !["checkbox", "radio", "range", "color", "file"].includes(el.type)) || el instanceof HTMLTextAreaElement) return null;
  return el;
}

function show(el: HTMLElement) {
  const text = textOf(el);
  if (!text) return;
  // Visible text that says the same thing needs no tip.
  if (text === el.textContent?.trim() && el.scrollWidth <= el.clientWidth) return;
  const title = el.getAttribute("title");
  if (title) {
    el.dataset.tipTitle = title;
    el.removeAttribute("title");
  }
  tip ??= Object.assign(document.createElement("div"), { className: "app-tip", role: "tooltip" });
  if (!tip.isConnected) document.body.append(tip);
  tip.textContent = text;
  tip.hidden = false;
  const r = el.getBoundingClientRect();
  const t = tip.getBoundingClientRect();
  const gap = 6;
  let top = r.bottom + gap;
  if (top + t.height > innerHeight - 4) top = r.top - gap - t.height;
  let left = r.left + r.width / 2 - t.width / 2;
  left = Math.max(4, Math.min(left, innerWidth - t.width - 4));
  tip.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
}

function hide() {
  clearTimeout(timer);
  if (current?.dataset.tipTitle != null) {
    current.setAttribute("title", current.dataset.tipTitle);
    delete current.dataset.tipTitle;
  }
  if (tip && !tip.hidden) {
    tip.hidden = true;
    lastHidden = performance.now();
  }
  current = null;
}

function enter(el: HTMLElement | null) {
  if (el === current) return;
  hide();
  if (!el) return;
  current = el;
  const warm = performance.now() - lastHidden < WARM;
  timer = window.setTimeout(() => current === el && el.isConnected && show(el), warm ? 0 : DELAY);
}

export function installTooltips() {
  if ((window as { __tips?: boolean }).__tips) return;
  (window as { __tips?: boolean }).__tips = true;
  document.addEventListener("pointerover", (e) => {
    if ((e as PointerEvent).pointerType === "touch") return;
    enter(targetOf(e.target));
  });
  document.addEventListener("pointerleave", hide, true);
  document.addEventListener("pointerdown", hide, true);
  document.addEventListener("keydown", (e) => e.key === "Escape" && hide(), true);
  document.addEventListener("scroll", hide, true);
  window.addEventListener("blur", hide);
  document.addEventListener("focusin", (e) => {
    if (e.target instanceof HTMLElement && e.target.matches(":focus-visible")) enter(targetOf(e.target));
  });
  document.addEventListener("focusout", hide);
}
