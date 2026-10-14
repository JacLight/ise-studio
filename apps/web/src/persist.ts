// What is open stays open: panels, tools, the selection, what is hidden and
// the tree's open rows survive a refresh. Kept per project, on this computer
// only (browser storage), and read back once when the page loads.

import { type Dispatch, type SetStateAction, useCallback, useState } from "react";
import { projectName } from "./engine";

const key = (name: string) => `ise.ui.${projectName || "default"}.${name}`;

function read<T>(name: string, initial: T): T {
  try {
    const raw = localStorage.getItem(key(name));
    return raw == null ? initial : (JSON.parse(raw) as T);
  } catch {
    return initial;
  }
}

function write(name: string, value: unknown) {
  try {
    if (value === null || value === undefined || value === false) localStorage.removeItem(key(name));
    else localStorage.setItem(key(name), JSON.stringify(value));
  } catch {
    /* private window, or storage full: it just won't be remembered */
  }
}

/** `useState` that is remembered across refreshes (JSON values). */
export function usePersisted<T>(name: string, initial: T): [T, Dispatch<SetStateAction<T>>] {
  const [value, setValue] = useState<T>(() => read(name, initial));
  const set = useCallback<Dispatch<SetStateAction<T>>>(
    (next) =>
      setValue((prev) => {
        const v = typeof next === "function" ? (next as (p: T) => T)(prev) : next;
        write(name, v);
        return v;
      }),
    [name],
  );
  return [value, set];
}

/** A remembered set of strings. */
export function usePersistedSet(name: string): [ReadonlySet<string>, Dispatch<SetStateAction<ReadonlySet<string>>>] {
  const [list, setList] = usePersisted<string[]>(name, []);
  const [value, setValue] = useState<ReadonlySet<string>>(() => new Set(list));
  const set = useCallback<Dispatch<SetStateAction<ReadonlySet<string>>>>(
    (next) =>
      setValue((prev) => {
        const v = typeof next === "function" ? (next as (p: ReadonlySet<string>) => ReadonlySet<string>)(prev) : next;
        setList([...v]);
        return v;
      }),
    [setList],
  );
  return [value, set];
}
