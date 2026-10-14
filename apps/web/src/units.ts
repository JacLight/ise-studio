// Display preferences: the unit lengths are shown and typed in, and how
// many decimals. The engine always works in millimetres; this is only how
// this person reads and writes them. Kept in browser storage (per viewer).

import { useSyncExternalStore } from "react";

export type LengthUnit = "mm" | "cm" | "m" | "in" | "ft";
export type DisplayPrefs = { length: LengthUnit; decimals: number; angle: "deg" };

export const UNIT_MM: Record<LengthUnit, number> = { mm: 1, cm: 10, m: 1000, in: 25.4, ft: 304.8 };
export const LENGTH_UNITS: { unit: LengthUnit; label: string }[] = [
  { unit: "mm", label: "Millimetres (mm)" },
  { unit: "cm", label: "Centimetres (cm)" },
  { unit: "m", label: "Metres (m)" },
  { unit: "in", label: "Inches (in)" },
  { unit: "ft", label: "Feet (ft)" },
];

const KEY = "ise.display.v1";
const DEFAULTS: DisplayPrefs = { length: "mm", decimals: 2, angle: "deg" };

function load(): DisplayPrefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!saved || typeof saved !== "object") return DEFAULTS;
    const p = { ...DEFAULTS, ...saved } as DisplayPrefs;
    if (!(p.length in UNIT_MM)) p.length = "mm";
    p.decimals = Math.max(0, Math.min(6, Math.round(Number(p.decimals) || 0)));
    return p;
  } catch {
    return DEFAULTS;
  }
}

let current = load();
const listeners = new Set<() => void>();

export const displayPrefs = () => current;

export function setDisplayPrefs(patch: Partial<DisplayPrefs>) {
  current = { ...current, ...patch };
  try {
    localStorage.setItem(KEY, JSON.stringify(current));
  } catch {
    // Storage unavailable: the choice lasts until the page closes.
  }
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** The display preferences, re-rendering when they change. */
export function useDisplayPrefs() {
  return useSyncExternalStore(subscribe, displayPrefs, displayPrefs);
}

/** A length (mm) as this person reads it: "12.5 mm", "0.492 in". */
export function showLength(mm: number, prefs: DisplayPrefs = current, withUnit = true) {
  const f = UNIT_MM[prefs.length];
  // Larger units need more places to say as much (0.5 mm is 0.0197 in).
  const extra = prefs.length === "m" || prefs.length === "ft" ? 2 : prefs.length === "in" || prefs.length === "cm" ? 1 : 0;
  const v = Number((mm / f).toFixed(prefs.decimals + extra));
  const text = v.toLocaleString("en-US", { maximumFractionDigits: prefs.decimals + extra });
  return withUnit ? `${text} ${prefs.length}` : text;
}
