// Safe arithmetic for value fields: numbers with units, + - * / and
// parentheses. A leading "+", "*" or "/" applies to the current value
// ("+10", "*2"). Results are in millimeters. No eval.

const UNITS: Record<string, number> = { mm: 1, cm: 10, m: 1000, in: 25.4, '"': 25.4, ft: 304.8, "'": 304.8 };

type Token = { kind: "num"; value: number } | { kind: "op"; value: string };

function tokenize(src: string, unitMm = 1): Token[] | null {
  const tokens: Token[] = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === " ") {
      i++;
    } else if ("+-*/()".includes(c)) {
      tokens.push({ kind: "op", value: c });
      i++;
    } else if (/[\d.]/.test(c)) {
      const m = /^(\d+\.?\d*|\.\d+)\s*(mm|cm|m|in|ft|"|')?/.exec(src.slice(i));
      if (!m) return null;
      // Bare numbers are in the field's unit; written units convert to it.
      tokens.push({ kind: "num", value: Number(m[1]) * (m[2] ? UNITS[m[2]] / unitMm : 1) });
      i += m[0].length;
    } else {
      return null;
    }
  }
  return tokens;
}

function parse(tokens: Token[]): number | null {
  let pos = 0;
  const peek = () => tokens[pos];
  const isOp = (v: string) => peek()?.kind === "op" && peek()!.value === v;

  function factor(): number | null {
    if (isOp("-") || isOp("+")) {
      const sign = (tokens[pos++] as { value: string }).value === "-" ? -1 : 1;
      const v = factor();
      return v === null ? null : sign * v;
    }
    if (isOp("(")) {
      pos++;
      const v = expr();
      if (!isOp(")")) return null;
      pos++;
      return v;
    }
    const t = peek();
    if (t?.kind === "num") {
      pos++;
      return t.value;
    }
    return null;
  }

  function term(): number | null {
    let v = factor();
    while (v !== null && (isOp("*") || isOp("/"))) {
      const op = (tokens[pos++] as { value: string }).value;
      const r = factor();
      if (r === null) return null;
      v = op === "*" ? v * r : v / r;
    }
    return v;
  }

  function expr(): number | null {
    let v = term();
    while (v !== null && (isOp("+") || isOp("-"))) {
      const op = (tokens[pos++] as { value: string }).value;
      const r = term();
      if (r === null) return null;
      v = op === "+" ? v + r : v - r;
    }
    return v;
  }

  const v = expr();
  return pos === tokens.length ? v : null;
}

/**
 * Evaluates `input` against `current` (mm). Returns null if invalid.
 * `unitMm` is the unit bare numbers are in (25.4 when lengths show in inches).
 */
export function evaluate(input: string, current: number, unitMm = 1): number | null {
  const src = input.trim().toLowerCase();
  if (!src) return null;
  const relative = /^[+*/]/.test(src) ? src[0] : null;
  // A scale (*2, /3) is a plain number, whatever the unit.
  const tokens = tokenize(relative ? src.slice(1) : src, relative === "*" || relative === "/" ? 1 : unitMm);
  if (!tokens || tokens.length === 0) return null;
  const v = parse(tokens);
  if (v === null || !Number.isFinite(v)) return null;
  if (relative === "+") return current + v * unitMm;
  if (relative === "*") return current * v;
  if (relative === "/") return v === 0 ? null : current / v;
  return v * unitMm;
}
