// OpenSCAD in Forma: a parser and evaluator for .scad source, loaded next to
// the prelude when a program calls openscad(). Imported files stay live:
// `return openscad(SOURCE, { width: p.width })` re-runs the file with the
// top-level values replaced by the shape's parameters, building with the
// prelude's own vocabulary (box, cylinder, sphere, polygons, extrude, revolve,
// booleans, hull, minkowski), so cubes, cylinders, spheres and extrusions stay
// exact in STEP.
//
// The language: modules and functions (recursive, with defaults and named
// arguments), children(), variables with OpenSCAD's scoping (the last
// assignment in a scope wins), $-variables passed down dynamically, if/else,
// for and intersection_for (ranges, lists, several variables), let, echo,
// assert, list comprehensions (for, if/else, let, each), function literals,
// ranges, vectors and matrices (+ − × with dot and matrix products), ?:, and
// the usual built-in functions. Geometry: cube, sphere, cylinder, polyhedron,
// square, circle, polygon, text (when the prelude has text), translate,
// rotate (angles or axis-angle), scale, resize, mirror, multmatrix, color,
// offset, hull, minkowski, union, difference, intersection, linear_extrude,
// rotate_extrude, render. $fn between 3 and 16 makes true polygons (a hex
// nut trap stays six-sided); otherwise curves are round, meshed to Forma's
// tolerance. import(), surface() and projection() are refused with a reason.

const __scad = (() => {
  class ScadError extends Error {}
  const fail = (msg, line) => {
    throw new ScadError(line ? `line ${line}: ${msg}` : msg);
  };

  // ---- Lexer ---------------------------------------------------------------------------

  function lex(src) {
    const toks = [];
    let i = 0;
    let line = 1;
    const ops3 = [];
    const ops2 = ["<=", ">=", "==", "!=", "&&", "||"];
    const single = "+-*/%^!<>=?:;,.()[]{}#";
    while (i < src.length) {
      const c = src[i];
      if (c === "\n") {
        line++;
        i++;
        continue;
      }
      if (/\s/.test(c)) {
        i++;
        continue;
      }
      if (c === "/" && src[i + 1] === "/") {
        const end = src.indexOf("\n", i);
        toks.push({ t: "comment", v: src.slice(i + 2, end < 0 ? src.length : end), line });
        i = end < 0 ? src.length : end;
        continue;
      }
      if (c === "/" && src[i + 1] === "*") {
        const end = src.indexOf("*/", i + 2);
        const body = src.slice(i, end < 0 ? src.length : end + 2);
        line += (body.match(/\n/g) || []).length;
        i = end < 0 ? src.length : end + 2;
        continue;
      }
      if (/[0-9]/.test(c) || (c === "." && /[0-9]/.test(src[i + 1] ?? ""))) {
        const m = /^(\d*\.?\d*(?:[eE][+-]?\d+)?)/.exec(src.slice(i));
        toks.push({ t: "num", v: Number(m[1]), line });
        i += m[1].length;
        continue;
      }
      if (c === '"') {
        let s = "";
        i++;
        while (i < src.length && src[i] !== '"') {
          if (src[i] === "\\") {
            const n = src[i + 1];
            s += n === "n" ? "\n" : n === "t" ? "\t" : n === "r" ? "\r" : n;
            i += 2;
          } else {
            if (src[i] === "\n") line++;
            s += src[i++];
          }
        }
        i++;
        toks.push({ t: "str", v: s, line });
        continue;
      }
      if (/[A-Za-z_$]/.test(c)) {
        const m = /^[A-Za-z_$][A-Za-z0-9_]*/.exec(src.slice(i));
        toks.push({ t: "id", v: m[0], line });
        i += m[0].length;
        // include <file> / use <file>
        if (m[0] === "include" || m[0] === "use") {
          const rest = /^\s*<([^>]*)>/.exec(src.slice(i));
          if (rest) {
            toks.push({ t: "path", v: rest[1], line });
            i += rest[0].length;
          }
        }
        continue;
      }
      const two = src.slice(i, i + 2);
      if (ops2.includes(two) || ops3.includes(two)) {
        toks.push({ t: "op", v: two, line });
        i += 2;
        continue;
      }
      if (single.includes(c)) {
        toks.push({ t: "op", v: c, line });
        i++;
        continue;
      }
      fail(`unexpected “${c}”`, line);
    }
    toks.push({ t: "eof", v: null, line });
    return toks;
  }

  // ---- Parser ----------------------------------------------------------------------------

  function parse(src) {
    const all = lex(src);
    const toks = all.filter((t) => t.t !== "comment");
    let at = 0;
    const peek = (k = 0) => toks[at + k];
    const next = () => toks[at++];
    const is = (v, k = 0) => peek(k).t === "op" && peek(k).v === v;
    const isId = (v, k = 0) => peek(k).t === "id" && peek(k).v === v;
    const expect = (v) => {
      const t = next();
      if (!(t.t === "op" && t.v === v)) fail(`expected “${v}” but found “${t.v ?? "the end"}”`, t.line);
      return t;
    };
    const ident = () => {
      const t = next();
      if (t.t !== "id") fail(`expected a name but found “${t.v ?? "the end"}”`, t.line);
      return t.v;
    };

    function statements(until) {
      const out = [];
      while (!(until ? is(until) : peek().t === "eof")) {
        if (peek().t === "eof") fail(`missing “${until}”`, peek().line);
        const s = statement();
        if (s) out.push(s);
      }
      return out;
    }

    function params() {
      expect("(");
      const out = [];
      while (!is(")")) {
        const name = ident();
        let def = null;
        if (is("=")) {
          next();
          def = expr();
        }
        out.push({ name, def });
        if (!is(")")) expect(",");
      }
      expect(")");
      return out;
    }

    function args() {
      expect("(");
      const out = [];
      while (!is(")")) {
        if (peek().t === "id" && is("=", 1)) {
          const name = ident();
          next();
          out.push({ name, value: expr() });
        } else out.push({ name: null, value: expr() });
        if (!is(")")) expect(",");
      }
      expect(")");
      return out;
    }

    function assignments() {
      expect("(");
      const out = [];
      while (!is(")")) {
        const name = ident();
        let value = null;
        if (is("=")) {
          next();
          value = expr();
        }
        out.push({ name, value });
        if (!is(")")) expect(",");
      }
      expect(")");
      return out;
    }

    function statement() {
      const t = peek();
      if (is(";")) {
        next();
        return null;
      }
      if (is("{")) {
        next();
        const body = statements("}");
        expect("}");
        return { k: "block", body, line: t.line };
      }
      if (t.t === "id" && (t.v === "include" || t.v === "use")) {
        next();
        const p = peek().t === "path" ? next().v : "";
        fail(`${t.v} <${p}>: Forma imports one file; paste the library's modules into it`, t.line);
      }
      if (isId("module")) {
        next();
        const name = ident();
        const ps = params();
        const body = statement();
        return { k: "module", name, params: ps, body, line: t.line };
      }
      if (isId("function")) {
        next();
        const name = ident();
        const ps = params();
        expect("=");
        const body = expr();
        expect(";");
        return { k: "function", name, params: ps, body, line: t.line };
      }
      if (t.t === "id" && is("=", 1)) {
        const name = ident();
        next();
        const value = expr();
        expect(";");
        return { k: "assign", name, value, line: t.line };
      }
      if (isId("if")) {
        next();
        expect("(");
        const cond = expr();
        expect(")");
        const then = statement();
        let other = null;
        if (isId("else")) {
          next();
          other = statement();
        }
        return { k: "if", cond, then, other, line: t.line };
      }
      if (isId("for") || isId("intersection_for")) {
        next();
        const vars = assignments();
        const body = statement();
        return { k: t.v === "for" ? "for" : "ifor", vars, body, line: t.line };
      }
      if (isId("let")) {
        next();
        const vars = assignments();
        const body = statement();
        return { k: "let", vars, body, line: t.line };
      }
      // Modifiers.
      if (t.t === "op" && "!#%*".includes(t.v)) {
        next();
        const s = statement();
        if (!s) return null;
        if (t.v === "*" || t.v === "%") return null; // disabled / background: not part of the model
        if (t.v === "!") return { ...s, root: true };
        return s;
      }
      if (t.t === "id") {
        const name = ident();
        const a = args();
        let children = [];
        if (is(";")) next();
        else {
          const c = statement();
          children = c ? (c.k === "block" ? c.body : [c]) : [];
        }
        return { k: "call", name, args: a, children, line: t.line };
      }
      fail(`unexpected “${t.v ?? "end of file"}”`, t.line);
    }

    // Expressions, lowest precedence first.
    function expr() {
      const t = peek();
      if (isId("function") && is("(", 1)) {
        next();
        const ps = params();
        const body = expr();
        return { k: "lambda", params: ps, body, line: t.line };
      }
      if (isId("let") && is("(", 1)) {
        next();
        const vars = assignments();
        return { k: "letx", vars, body: expr(), line: t.line };
      }
      if ((isId("assert") || isId("echo")) && is("(", 1)) {
        next();
        const a = args();
        const body = is(")") || is(";") || is(",") || is("]") ? null : expr();
        return { k: t.v === "assert" ? "assertx" : "echox", args: a, body, line: t.line };
      }
      const c = or();
      if (is("?")) {
        next();
        const a = expr();
        expect(":");
        const b = expr();
        return { k: "cond", c, a, b, line: t.line };
      }
      return c;
    }
    const binary = (sub, ops) => () => {
      let left = sub();
      while (peek().t === "op" && ops.includes(peek().v)) {
        const op = next();
        left = { k: "bin", op: op.v, a: left, b: sub(), line: op.line };
      }
      return left;
    };
    function unary() {
      if (is("!") || is("-") || is("+")) {
        const op = next();
        return { k: "un", op: op.v, a: unary(), line: op.line };
      }
      return power();
    }
    function power() {
      const base = postfix();
      if (is("^")) {
        const op = next();
        return { k: "bin", op: "^", a: base, b: unary(), line: op.line };
      }
      return base;
    }
    const mul = binary(unary, ["*", "/", "%"]);
    const add = binary(mul, ["+", "-"]);
    const rel = binary(add, ["<", "<=", ">", ">="]);
    const eq = binary(rel, ["==", "!="]);
    const and = binary(eq, ["&&"]);
    const or = binary(and, ["||"]);

    function postfix() {
      let e = primary();
      for (;;) {
        if (is("(")) {
          const line = peek().line;
          e = { k: "callx", f: e, args: args(), line };
        } else if (is("[")) {
          const line = next().line;
          const i = expr();
          expect("]");
          e = { k: "index", a: e, i, line };
        } else if (is(".")) {
          const line = next().line;
          e = { k: "member", a: e, m: ident(), line };
        } else return e;
      }
    }

    function primary() {
      const t = next();
      if (t.t === "num") return { k: "num", v: t.v };
      if (t.t === "str") return { k: "str", v: t.v };
      if (t.t === "id") {
        if (t.v === "true") return { k: "num", v: true };
        if (t.v === "false") return { k: "num", v: false };
        if (t.v === "undef") return { k: "num", v: undefined };
        return { k: "var", name: t.v, line: t.line };
      }
      if (t.t === "op" && t.v === "(") {
        const e = expr();
        expect(")");
        return e;
      }
      if (t.t === "op" && t.v === "[") {
        if (is("]")) {
          next();
          return { k: "list", items: [] };
        }
        const first = element();
        if (is(":") && first.k !== "lc") {
          next();
          const second = expr();
          if (is(":")) {
            next();
            const third = expr();
            expect("]");
            return { k: "range", a: first, s: second, b: third, line: t.line };
          }
          expect("]");
          return { k: "range", a: first, s: null, b: second, line: t.line };
        }
        const items = [first];
        while (is(",")) {
          next();
          if (is("]")) break;
          items.push(element());
        }
        expect("]");
        return { k: "list", items };
      }
      fail(`unexpected “${t.v ?? "end of file"}”`, t.line);
    }

    // A list element: an expression or a comprehension.
    function element() {
      const t = peek();
      if (isId("for") && is("(", 1)) {
        next();
        // for (init; cond; update) C-style, or for (vars)
        expect("(");
        const save = at;
        // detect C-style by scanning for ';' before matching ')'
        let depth = 0;
        let cstyle = false;
        for (let k = at; k < toks.length; k++) {
          const x = toks[k];
          if (x.t === "op" && (x.v === "(" || x.v === "[")) depth++;
          if (x.t === "op" && (x.v === ")" || x.v === "]")) {
            if (depth === 0) break;
            depth--;
          }
          if (depth === 0 && x.t === "op" && x.v === ";") {
            cstyle = true;
            break;
          }
        }
        at = save - 1;
        if (cstyle) {
          expect("(");
          const init = [];
          while (!is(";")) {
            const name = ident();
            expect("=");
            init.push({ name, value: expr() });
            if (is(",")) next();
          }
          expect(";");
          const cond = expr();
          expect(";");
          const update = [];
          while (!is(")")) {
            const name = ident();
            expect("=");
            update.push({ name, value: expr() });
            if (is(",")) next();
          }
          expect(")");
          return { k: "lc", lc: "cfor", init, cond, update, body: element(), line: t.line };
        }
        const vars = assignments();
        return { k: "lc", lc: "for", vars, body: element(), line: t.line };
      }
      if (isId("if") && is("(", 1)) {
        next();
        expect("(");
        const cond = expr();
        expect(")");
        const then = element();
        let other = null;
        if (isId("else")) {
          next();
          other = element();
        }
        return { k: "lc", lc: "if", cond, then, other, line: t.line };
      }
      if (isId("let") && is("(", 1)) {
        next();
        const vars = assignments();
        return { k: "lc", lc: "let", vars, body: element(), line: t.line };
      }
      if (isId("each")) {
        next();
        return { k: "lc", lc: "each", body: element(), line: t.line };
      }
      if (is("(") && (isId("for", 1) || isId("if", 1) || isId("let", 1) || isId("each", 1))) {
        next();
        const e = element();
        expect(")");
        return e;
      }
      return expr();
    }

    const body = statements(null);
    return { body, comments: all.filter((t) => t.t === "comment"), toks: all };
  }

  // ---- Values ---------------------------------------------------------------------------------

  const isNum = (v) => typeof v === "number";
  const isList = (v) => Array.isArray(v);
  const isRange = (v) => v instanceof Range;
  class Range {
    constructor(a, s, b) {
      this.a = a;
      this.s = s;
      this.b = b;
    }
    *[Symbol.iterator]() {
      const { a, s, b } = this;
      if (!(isNum(a) && isNum(s) && isNum(b)) || s === 0) return;
      if ((s > 0 && a > b) || (s < 0 && a < b)) return;
      const n = Math.floor((b - a) / s + 1e-9);
      if (n > 1e6) fail("a range with more than a million steps");
      for (let k = 0; k <= n; k++) yield a + k * s;
    }
  }
  class Func {
    constructor(params, body, env, name) {
      this.params = params;
      this.body = body;
      this.env = env;
      this.name = name;
    }
  }
  const truthy = (v) => !(v === undefined || v === false || v === 0 || v === "" || (isList(v) && v.length === 0) || Number.isNaN(v));
  const equal = (a, b) => {
    if (isList(a) && isList(b)) return a.length === b.length && a.every((x, i) => equal(x, b[i]));
    return a === b;
  };
  const listOf = (v) => (isRange(v) ? [...v] : v);

  function arith(op, a, b) {
    a = listOf(a);
    b = listOf(b);
    if (isNum(a) && isNum(b)) {
      switch (op) {
        case "+":
          return a + b;
        case "-":
          return a - b;
        case "*":
          return a * b;
        case "/":
          return a / b;
        case "%":
          return a % b;
        case "^":
          return Math.pow(a, b);
      }
    }
    if (op === "+" || op === "-") {
      if (isList(a) && isList(b)) {
        const n = Math.min(a.length, b.length);
        const out = [];
        for (let i = 0; i < n; i++) out.push(arith(op, a[i], b[i]));
        return out;
      }
      return undefined;
    }
    if (op === "*") {
      if (isNum(a) && isList(b)) return b.map((x) => arith("*", a, x));
      if (isList(a) && isNum(b)) return a.map((x) => arith("*", x, b));
      if (isList(a) && isList(b)) {
        const am = a.length && isList(a[0]);
        const bm = b.length && isList(b[0]);
        if (!am && !bm) {
          // dot product
          if (a.length !== b.length) return undefined;
          let s = 0;
          for (let i = 0; i < a.length; i++) s += a[i] * b[i];
          return s;
        }
        if (am && !bm) return a.map((row) => arith("*", row, b));
        if (!am && bm) {
          // vector × matrix
          const cols = b[0].length;
          const out = [];
          for (let j = 0; j < cols; j++) {
            let s = 0;
            for (let i = 0; i < a.length; i++) s += a[i] * b[i][j];
            out.push(s);
          }
          return out;
        }
        // matrix × matrix
        const cols = b[0].length;
        return a.map((row) => {
          const out = [];
          for (let j = 0; j < cols; j++) {
            let s = 0;
            for (let i = 0; i < row.length; i++) s += row[i] * b[i][j];
            out.push(s);
          }
          return out;
        });
      }
    }
    if (op === "/" && isList(a) && isNum(b)) return a.map((x) => arith("/", x, b));
    if (op === "/" && isNum(a) && isList(b)) return b.map((x) => arith("/", a, x));
    return undefined;
  }

  function compare(op, a, b) {
    if ((isNum(a) || typeof a === "boolean") && (isNum(b) || typeof b === "boolean")) {
      a = Number(a);
      b = Number(b);
    } else if (!(typeof a === "string" && typeof b === "string")) return undefined;
    switch (op) {
      case "<":
        return a < b;
      case "<=":
        return a <= b;
      case ">":
        return a > b;
      case ">=":
        return a >= b;
    }
  }

  function str(v) {
    if (v === undefined) return "undef";
    if (typeof v === "string") return v;
    if (isList(v)) return "[" + v.map((x) => (typeof x === "string" ? JSON.stringify(x) : str(x))).join(", ") + "]";
    if (isRange(v)) return `[${str(v.a)} : ${str(v.s)} : ${str(v.b)}]`;
    if (v instanceof Func) return "function";
    if (isNum(v)) return String(Number(v.toPrecision(12)));
    return String(v);
  }

  // ---- Environments -----------------------------------------------------------------------

  class Env {
    constructor(parent, caller) {
      this.parent = parent;
      this.caller = caller ?? parent;
      this.vars = new Map();
      this.funcs = new Map();
      this.mods = new Map();
      this.childrenOf = parent ? parent.childrenOf : null;
    }
    get(name, line) {
      if (name.startsWith("$")) {
        for (let e = this; e; e = e.caller) if (e.vars.has(name)) return e.vars.get(name);
        return DEFAULT_SPECIAL[name];
      }
      for (let e = this; e; e = e.parent) if (e.vars.has(name)) return e.vars.get(name);
      // A function name used as a value.
      for (let e = this; e; e = e.parent) if (e.funcs.has(name)) return e.funcs.get(name);
      if (BUILTIN_FUNCS[name]) return (...a) => BUILTIN_FUNCS[name](a.map((x) => x));
      if (name === "PI") return Math.PI;
      return undefined;
    }
    func(name) {
      for (let e = this; e; e = e.parent) {
        if (e.funcs.has(name)) return e.funcs.get(name);
        if (e.vars.has(name) && e.vars.get(name) instanceof Func) return e.vars.get(name);
      }
      return null;
    }
    mod(name) {
      for (let e = this; e; e = e.parent) if (e.mods.has(name)) return e.mods.get(name);
      return null;
    }
  }
  const DEFAULT_SPECIAL = { $fn: 0, $fa: 12, $fs: 2, $t: 0, $preview: false, $children: 0, $vpr: [55, 0, 25], $vpt: [0, 0, 0], $vpd: 140, $vpf: 22.5 };

  // ---- Built-in functions --------------------------------------------------------------------

  const D = Math.PI / 180;
  const num = (v, what) => {
    if (!isNum(v)) fail(`${what} must be a number (got ${str(v)})`);
    return v;
  };
  const BUILTIN_FUNCS = {
    abs: ([x]) => (isNum(x) ? Math.abs(x) : undefined),
    sign: ([x]) => (isNum(x) ? Math.sign(x) : undefined),
    sin: ([x]) => (isNum(x) ? Math.sin(x * D) : undefined),
    cos: ([x]) => (isNum(x) ? Math.cos(x * D) : undefined),
    tan: ([x]) => (isNum(x) ? Math.tan(x * D) : undefined),
    asin: ([x]) => (isNum(x) ? Math.asin(x) / D : undefined),
    acos: ([x]) => (isNum(x) ? Math.acos(x) / D : undefined),
    atan: ([x]) => (isNum(x) ? Math.atan(x) / D : undefined),
    atan2: ([y, x]) => (isNum(x) && isNum(y) ? Math.atan2(y, x) / D : undefined),
    floor: ([x]) => (isNum(x) ? Math.floor(x) : undefined),
    ceil: ([x]) => (isNum(x) ? Math.ceil(x) : undefined),
    round: ([x]) => (isNum(x) ? (x < 0 ? -Math.round(-x) : Math.round(x)) : undefined),
    ln: ([x]) => (isNum(x) ? Math.log(x) : undefined),
    log: ([a, b]) => (isNum(a) && b === undefined ? Math.log10(a) : isNum(a) && isNum(b) ? Math.log(b) / Math.log(a) : undefined),
    exp: ([x]) => (isNum(x) ? Math.exp(x) : undefined),
    pow: ([a, b]) => (isNum(a) && isNum(b) ? Math.pow(a, b) : undefined),
    sqrt: ([x]) => (isNum(x) ? Math.sqrt(x) : undefined),
    len: ([v]) => (isList(v) || typeof v === "string" ? v.length : isRange(v) ? [...v].length : undefined),
    min: (a) => {
      const v = a.length === 1 && isList(a[0]) ? a[0] : a;
      return v.length && v.every(isNum) ? Math.min(...v) : undefined;
    },
    max: (a) => {
      const v = a.length === 1 && isList(a[0]) ? a[0] : a;
      return v.length && v.every(isNum) ? Math.max(...v) : undefined;
    },
    norm: ([v]) => (isList(v) && v.every(isNum) ? Math.hypot(...v) : undefined),
    cross: ([a, b]) => {
      if (!isList(a) || !isList(b)) return undefined;
      if (a.length === 2 && b.length === 2) return a[0] * b[1] - a[1] * b[0];
      return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    },
    concat: (a) => a.flatMap((x) => (isList(x) ? x : isRange(x) ? [...x] : [x])),
    lookup: ([k, table]) => {
      if (!isNum(k) || !isList(table) || !table.length) return undefined;
      const t = [...table].sort((p, q) => p[0] - q[0]);
      if (k <= t[0][0]) return t[0][1];
      if (k >= t[t.length - 1][0]) return t[t.length - 1][1];
      for (let i = 1; i < t.length; i++) {
        if (k <= t[i][0]) {
          const [x0, y0] = t[i - 1];
          const [x1, y1] = t[i];
          return y0 + ((k - x0) / (x1 - x0)) * (y1 - y0);
        }
      }
    },
    str: (a) => a.map(str).join(""),
    chr: (a) => a.flatMap((x) => (isList(x) ? x : [x])).map((c) => String.fromCodePoint(c)).join(""),
    ord: ([s]) => (typeof s === "string" && s.length ? s.codePointAt(0) : undefined),
    search: ([needle, hay, num = 1, col = 0]) => {
      // The common forms: a value in a list (or list of rows by column), characters of a string.
      const rows = typeof hay === "string" ? [...hay] : listOf(hay);
      if (!isList(rows)) return undefined;
      const find = (x) => {
        const hits = [];
        rows.forEach((r, i) => {
          const v = isList(r) ? r[col] : r;
          if (equal(v, x) && (num === 0 || hits.length < num)) hits.push(i);
        });
        return num === 1 ? (hits.length ? hits[0] : []) : hits;
      };
      if (typeof needle === "string" && typeof hay !== "string") return [...needle].map(find);
      if (isList(needle)) return needle.map(find);
      return num === 1 ? (find(needle) === undefined ? [] : [find(needle)].flat()) : find(needle);
    },
    rands: ([lo, hi, n, seed]) => {
      const r = rng(isNum(seed) ? seed : 1);
      return Array.from({ length: Math.max(0, n | 0) }, () => lo + r() * (hi - lo));
    },
    version: () => [2021, 1, 0],
    version_num: () => 20210100,
    is_undef: ([x]) => x === undefined,
    is_num: ([x]) => isNum(x) && !Number.isNaN(x),
    is_bool: ([x]) => typeof x === "boolean",
    is_string: ([x]) => typeof x === "string",
    is_list: ([x]) => isList(x),
    is_function: ([x]) => x instanceof Func || typeof x === "function",
  };

  // ---- Evaluation -------------------------------------------------------------------------------

  let depth = 0;
  const messages = [];

  function evalExpr(e, env) {
    switch (e.k) {
      case "num":
      case "str":
        return e.v;
      case "var":
        return env.get(e.name, e.line);
      case "un": {
        const a = evalExpr(e.a, env);
        if (e.op === "!") return !truthy(a);
        if (e.op === "-") return isNum(a) ? -a : isList(a) || isRange(a) ? arith("*", -1, a) : undefined;
        return a;
      }
      case "bin": {
        if (e.op === "&&") return truthy(evalExpr(e.a, env)) && truthy(evalExpr(e.b, env));
        if (e.op === "||") return truthy(evalExpr(e.a, env)) || truthy(evalExpr(e.b, env));
        const a = evalExpr(e.a, env);
        const b = evalExpr(e.b, env);
        if (e.op === "==") return equal(a, b);
        if (e.op === "!=") return !equal(a, b);
        if (["<", "<=", ">", ">="].includes(e.op)) return compare(e.op, a, b);
        return arith(e.op, a, b);
      }
      case "cond":
        return truthy(evalExpr(e.c, env)) ? evalExpr(e.a, env) : evalExpr(e.b, env);
      case "index": {
        const a = listOf(evalExpr(e.a, env));
        const i = evalExpr(e.i, env);
        if (!isNum(i) || !(isList(a) || typeof a === "string")) return undefined;
        return a[Math.floor(i)];
      }
      case "member": {
        const a = evalExpr(e.a, env);
        const i = { x: 0, y: 1, z: 2 }[e.m];
        return isList(a) && i !== undefined ? a[i] : undefined;
      }
      case "range": {
        const a = evalExpr(e.a, env);
        const s = e.s ? evalExpr(e.s, env) : 1;
        const b = evalExpr(e.b, env);
        return new Range(a, s, b);
      }
      case "list": {
        const out = [];
        for (const item of e.items) {
          if (item.k === "lc") out.push(...comprehension(item, env));
          else out.push(evalExpr(item, env));
        }
        return out;
      }
      case "lc":
        return comprehension(e, env);
      case "lambda":
        return new Func(e.params, e.body, env, "function");
      case "letx": {
        const inner = new Env(env);
        for (const v of e.vars) inner.vars.set(v.name, v.value ? evalExpr(v.value, inner) : undefined);
        return evalExpr(e.body, inner);
      }
      case "assertx": {
        const [c, msg] = e.args.map((a) => evalExpr(a.value, env));
        if (!truthy(c)) fail(`assertion failed${msg !== undefined ? `: ${str(msg)}` : ""}`, e.line);
        return e.body ? evalExpr(e.body, env) : undefined;
      }
      case "echox":
        messages.push(e.args.map((a) => (a.name ? `${a.name} = ` : "") + str(evalExpr(a.value, env))).join(", "));
        return e.body ? evalExpr(e.body, env) : undefined;
      case "callx":
        return callFunction(e, env);
    }
    fail(`cannot work out ${e.k}`, e.line);
  }

  function comprehension(e, env) {
    switch (e.lc) {
      case "for": {
        const out = [];
        iterate(e.vars, env, (inner) => {
          if (e.body.k === "lc") out.push(...comprehension(e.body, inner));
          else out.push(evalExpr(e.body, inner));
        });
        return out;
      }
      case "cfor": {
        const out = [];
        let inner = new Env(env);
        for (const v of e.init) inner.vars.set(v.name, evalExpr(v.value, inner));
        let guard = 0;
        while (truthy(evalExpr(e.cond, inner))) {
          if (++guard > 1e6) fail("a for loop that does not end", e.line);
          if (e.body.k === "lc") out.push(...comprehension(e.body, inner));
          else out.push(evalExpr(e.body, inner));
          const nextEnv = new Env(env);
          for (const v of e.update) nextEnv.vars.set(v.name, evalExpr(v.value, inner));
          for (const [k, v] of inner.vars) if (!nextEnv.vars.has(k)) nextEnv.vars.set(k, v);
          inner = nextEnv;
        }
        return out;
      }
      case "if": {
        const branch = truthy(evalExpr(e.cond, env)) ? e.then : e.other;
        if (!branch) return [];
        return branch.k === "lc" ? comprehension(branch, env) : [evalExpr(branch, env)];
      }
      case "let": {
        const inner = new Env(env);
        for (const v of e.vars) inner.vars.set(v.name, v.value ? evalExpr(v.value, inner) : undefined);
        return e.body.k === "lc" ? comprehension(e.body, inner) : [evalExpr(e.body, inner)];
      }
      case "each": {
        const v = e.body.k === "lc" ? comprehension(e.body, env) : evalExpr(e.body, env);
        return isList(v) ? v : isRange(v) ? [...v] : typeof v === "string" ? [...v] : [v];
      }
    }
  }

  /** Runs `body` for every combination of the loop variables (nested, first outermost). */
  function iterate(vars, env, body) {
    const step = (k, inner) => {
      if (k === vars.length) return body(inner);
      const v = vars[k];
      let over = evalExpr(v.value, inner);
      if (isNum(over) || typeof over === "boolean") over = [over];
      else if (typeof over === "string") over = [...over];
      else if (over === undefined) return;
      let count = 0;
      for (const x of over) {
        if (++count > 1e6) fail("a loop over more than a million values");
        const next = new Env(inner);
        next.vars.set(v.name, x);
        step(k + 1, next);
      }
    };
    step(0, env);
  }

  function bind(params, args, defEnv, callEnv, line) {
    const env = new Env(defEnv, callEnv);
    const given = new Map();
    let pos = 0;
    for (const a of args) {
      if (a.name) given.set(a.name, a.value);
      else if (pos < params.length) given.set(params[pos++].name, a.value);
    }
    for (const p of params) {
      if (given.has(p.name)) env.vars.set(p.name, given.get(p.name));
      else env.vars.set(p.name, p.def ? evalExpr(p.def, env) : undefined);
    }
    // $variables given at the call are seen inside.
    for (const [k, v] of given) if (k.startsWith("$")) env.vars.set(k, v);
    return env;
  }

  function evalArgs(args, env) {
    return args.map((a) => ({ name: a.name, value: evalExpr(a.value, env) }));
  }

  function callFunction(e, env) {
    let f = null;
    let name = null;
    if (e.f.k === "var") {
      name = e.f.name;
      f = env.func(name);
      if (!f && BUILTIN_FUNCS[name]) return BUILTIN_FUNCS[name](evalArgs(e.args, env).map((x) => x.value));
      if (!f) f = env.get(name);
    } else f = evalExpr(e.f, env);
    if (typeof f === "function") return f(...evalArgs(e.args, env).map((x) => x.value));
    if (!(f instanceof Func)) {
      if (name && !f) fail(`unknown function ${name}()`, e.line);
      return undefined;
    }
    if (++depth > 3000) fail("functions call themselves too deeply (over 3000)", e.line);
    try {
      const inner = bind(f.params, evalArgs(e.args, env), f.env, env, e.line);
      return evalExpr(f.body, inner);
    } finally {
      depth--;
    }
  }

  // ---- Geometry -------------------------------------------------------------------------------------

  // A geometry is { d: 3, s: Solid } or { d: 2, p: Profile }, with the colour it was given.
  const g3 = (s, color) => ({ d: 3, s, color });
  const g2 = (p, color, circle) => ({ d: 2, p, color, circle });

  /** One solid (and/or one profile) from a group of geometries, kept apart by colour. */
  function unite(list) {
    const out = [];
    const keys = new Map();
    for (const g of list) {
      const key = `${g.d}|${g.color ?? ""}`;
      if (!keys.has(key)) keys.set(key, []);
      keys.get(key).push(g);
    }
    for (const [, gs] of keys) {
      if (gs.length === 1) out.push(gs[0]);
      else if (gs[0].d === 3) out.push(g3(union(gs.map((g) => g.s)), gs[0].color));
      else out.push(g2(gs.slice(1).reduce((a, g) => a.add(g.p), gs[0].p), gs[0].color));
    }
    return out;
  }
  /** Everything in a group as one geometry of each kind, colour dropped. */
  function merge(list) {
    const s = list.filter((g) => g.d === 3);
    const p = list.filter((g) => g.d === 2);
    const out = [];
    if (s.length) out.push(g3(s.length === 1 ? s[0].s : union(s.map((g) => g.s)), s[0].color));
    if (p.length) out.push(g2(p.slice(1).reduce((a, g) => a.add(g.p), p[0].p), p[0].color, p.length === 1 ? p[0].circle : undefined));
    return out;
  }

  const vecArg = (v, n, fill, what) => {
    if (isNum(v)) return Array(n).fill(v);
    if (!isList(v)) fail(`${what} must be a number or a vector`);
    return Array.from({ length: n }, (_, i) => (isNum(v[i]) ? v[i] : fill));
  };

  /** Segments for a curve of radius r, or null for a true circle. */
  function polySides(env, r) {
    const fn = env.get("$fn");
    if (isNum(fn) && fn >= 3 && fn <= 16) return Math.floor(fn);
    return null;
  }

  function transformMatrix(m) {
    // OpenSCAD's 4×4 (or 3×4) row-major matrix → prelude's 3×4.
    const r = (i) => (isList(m[i]) ? m[i] : [0, 0, 0, 0]);
    const rows = [r(0), r(1), r(2)].map((row) => [0, 1, 2, 3].map((j) => (isNum(row[j]) ? row[j] : 0)));
    if (!isList(m[0])) fail("multmatrix needs a matrix");
    if (!isList(m[1])) rows[1] = [0, 1, 0, 0];
    if (!isList(m[2])) rows[2] = [0, 0, 1, 0];
    return rows;
  }

  function applyMatrix(g, rows) {
    if (g.d === 3) return g3(new Solid(__n.sop("transform", g.s.h, rows.flat())), g.color);
    // 2D: rotation, mirror and scale without shear, then a move.
    const [a, b, , tx] = rows[0];
    const [c, d, , ty] = rows[1];
    const sx = Math.hypot(a, c);
    const sy = Math.hypot(b, d);
    if (Math.abs(a * b + c * d) > 1e-9 * Math.max(1, sx * sy)) fail("multmatrix on a 2D shape: shear is not supported");
    const det = a * d - b * c;
    let p = g.p;
    if (det < 0) p = p.mirror(0, 1);
    p = p.scale(sx, sy);
    const ang = Math.atan2(c, a) / D;
    if (ang) p = p.rotate(ang);
    if (tx || ty) p = p.translate(tx, ty);
    return g2(p, g.color);
  }

  function axisAngle(a, v) {
    const n = Math.hypot(v[0], v[1], v[2]);
    if (!n) return null;
    const [x, y, z] = v.map((c) => c / n);
    const c = Math.cos(a * D);
    const s = Math.sin(a * D);
    const t = 1 - c;
    return [
      [t * x * x + c, t * x * y - s * z, t * x * z + s * y, 0],
      [t * x * y + s * z, t * y * y + c, t * y * z - s * x, 0],
      [t * x * z - s * y, t * y * z + s * x, t * z * z + c, 0],
    ];
  }

  function named(args, names) {
    const out = {};
    let pos = 0;
    for (const a of args) {
      if (a.name) out[a.name] = a.value;
      else if (pos < names.length) out[names[pos++]] = a.value;
    }
    return out;
  }

  const MODULES = {
    cube(a) {
      const { size = 1, center = false } = named(a, ["size", "center"]);
      const [x, y, z] = vecArg(size, 3, 1, "cube size");
      if (x <= 0 || y <= 0 || z <= 0) return [];
      return [g3(box(x, y, z, { center: truthy(center) }))];
    },
    sphere(a, env) {
      const o = named(a, ["r"]);
      const r = isNum(o.d) ? o.d / 2 : isNum(o.r) ? o.r : 1;
      if (r <= 0) return [];
      return [g3(sphere(r))];
    },
    cylinder(a, env) {
      const o = named(a, ["h", "r1", "r2", "center"]);
      const h = isNum(o.h) ? o.h : 1;
      let r1 = isNum(o.r) ? o.r : 1;
      let r2 = r1;
      if (isNum(o.d)) r1 = r2 = o.d / 2;
      if (isNum(o.r1)) r1 = o.r1;
      if (isNum(o.r2)) r2 = o.r2;
      if (isNum(o.d1)) r1 = o.d1 / 2;
      if (isNum(o.d2)) r2 = o.d2 / 2;
      if (h <= 0 || (r1 <= 0 && r2 <= 0)) return [];
      const center = truthy(o.center);
      const n = polySides(env, Math.max(r1, r2));
      if (n) {
        // A true prism or frustum: what $fn = 6 asks for.
        let s = r1 > 0 ? regularPolygon(n, r1).extrude(h, { scaleTop: Math.max(r2 / r1, 1e-3) }) : regularPolygon(n, r2).extrude(h, { scaleTop: 1e-3 }).mirror(0, 0, 1).translate(0, 0, h);
        return [g3(center ? s.translate(0, 0, -h / 2) : s)];
      }
      return [g3(cylinder(h, Math.max(r1, 0), { top: Math.max(r2, 0), center }))];
    },
    polyhedron(a) {
      const o = named(a, ["points", "faces", "convexity"]);
      const pts = o.points;
      const faces = o.faces ?? o.triangles;
      if (!isList(pts) || !isList(faces)) fail("polyhedron needs points and faces");
      const tris = [];
      for (const f of faces) {
        if (!isList(f) || f.length < 3) continue;
        // OpenSCAD faces run clockwise seen from outside; Forma's run counter-clockwise.
        for (let i = 1; i + 1 < f.length; i++) tris.push([f[0], f[i + 1], f[i]]);
      }
      return [g3(fromMesh(pts.map((p) => vecArg(p, 3, 0, "a point")), tris))];
    },
    square(a) {
      const { size = 1, center = false } = named(a, ["size", "center"]);
      const [x, y] = vecArg(size, 2, 1, "square size");
      if (x <= 0 || y <= 0) return [];
      return [g2(rect(x, y, { center: truthy(center) }))];
    },
    circle(a, env) {
      const o = named(a, ["r"]);
      const r = isNum(o.d) ? o.d / 2 : isNum(o.r) ? o.r : 1;
      if (r <= 0) return [];
      const n = polySides(env, r);
      return [n ? g2(regularPolygon(n, r)) : g2(circle(r), undefined, r)];
    },
    polygon(a) {
      const o = named(a, ["points", "paths", "convexity"]);
      if (!isList(o.points) || o.points.length < 3) fail("polygon needs at least three points");
      const pts = o.points.map((p) => vecArg(p, 2, 0, "a point"));
      const paths = isList(o.paths) && o.paths.length ? o.paths.map((path) => path.map((i) => pts[i])) : [pts];
      // The first outline adds area, the others cut holes (as OpenSCAD reads nested paths).
      const contours = paths.map((c, i) => {
        const ccw = windingArea(c) >= 0;
        return (i === 0) === ccw ? c : [...c].reverse();
      });
      return [g2(polygons(contours))];
    },
    text(a, env) {
      if (typeof globalThis.text !== "function") fail("text(): this engine has no fonts yet; turn the lettering into polygons or leave it out");
      const o = named(a, ["text", "size", "font", "halign", "valign", "spacing", "direction", "language", "script"]);
      // OpenSCAD's size is about the cap height; Forma's text() takes the same em-ish size.
      const valign = { top: "top", center: "middle", bottom: "bottom", baseline: "baseline" }[o.valign] ?? "baseline";
      return [g2(globalThis.text(str(o.text ?? ""), { size: isNum(o.size) ? o.size : 10, align: ["left", "center", "right"].includes(o.halign) ? o.halign : "left", valign }))];
    },
    import() {
      fail("import(): Forma reads one .scad file; bring the STL or DXF in as its own part");
    },
    surface() {
      fail("surface(): height maps are not read from .scad files");
    },
    projection() {
      fail("projection(): use a sketch's projection in Forma instead (sketch.project)");
    },
  };

  // Operations on their children: (args, groups, env) → geometries.
  const OPS = {
    union: (a, groups) => unite(groups.flat()),
    group: (a, groups) => groups.flat(),
    render: (a, groups) => groups.flat(),
    difference(a, groups) {
      const first = merge(groups[0] ?? []);
      const rest = merge(groups.slice(1).flat());
      return first.map((g) => {
        const cut = rest.find((r) => r.d === g.d);
        if (!cut) return g;
        return g.d === 3 ? g3(g.s.subtract(cut.s), g.color) : g2(g.p.subtract(cut.p), g.color);
      });
    },
    intersection(a, groups) {
      const gs = groups.map(merge).filter((x) => x.length);
      if (!gs.length) return [];
      let acc = gs[0];
      for (const next of gs.slice(1)) {
        acc = acc
          .map((g) => {
            const o = next.find((r) => r.d === g.d);
            if (!o) return null;
            return g.d === 3 ? g3(g.s.intersect(o.s), g.color) : g2(g.p.intersect(o.p), g.color);
          })
          .filter(Boolean);
      }
      return acc;
    },
    translate(a, groups) {
      const { v = [0, 0, 0] } = named(a, ["v"]);
      const [x, y, z] = vecArg(v, 3, 0, "translate");
      return groups.flat().map((g) => (g.d === 3 ? g3(g.s.translate(x, y, z), g.color) : g2(g.p.translate(x, y), g.color)));
    },
    rotate(a, groups) {
      const o = named(a, ["a", "v"]);
      const ang = o.a ?? 0;
      const out = groups.flat();
      if (isList(ang)) {
        const [x, y, z] = vecArg(ang, 3, 0, "rotate");
        return out.map((g) => (g.d === 3 ? g3(g.s.rotate(x, y, z), g.color) : g2(g.p.rotate(z), g.color)));
      }
      num(ang, "rotate angle");
      if (isList(o.v)) {
        const v = vecArg(o.v, 3, 0, "rotate axis");
        const m = axisAngle(ang, v);
        if (!m) return out;
        if (Math.abs(v[0]) < 1e-12 && Math.abs(v[1]) < 1e-12) return out.map((g) => (g.d === 3 ? g3(g.s.rotate(0, 0, Math.sign(v[2]) * ang), g.color) : g2(g.p.rotate(Math.sign(v[2]) * ang), g.color)));
        return out.map((g) => applyMatrix(g, m));
      }
      return out.map((g) => (g.d === 3 ? g3(g.s.rotate(0, 0, ang), g.color) : g2(g.p.rotate(ang), g.color)));
    },
    scale(a, groups) {
      const { v = [1, 1, 1] } = named(a, ["v"]);
      const [x, y, z] = vecArg(v, 3, 1, "scale");
      return groups.flat().map((g) => (g.d === 3 ? g3(g.s.scale(x, y, z), g.color) : g2(g.p.scale(x, y), g.color)));
    },
    resize(a, groups) {
      const o = named(a, ["newsize", "auto"]);
      const size = vecArg(o.newsize ?? [0, 0, 0], 3, 0, "resize");
      const auto = isList(o.auto) ? o.auto.map(truthy) : Array(3).fill(truthy(o.auto));
      return merge(groups.flat()).map((g) => {
        const b = g.d === 3 ? g.s.bounds : g.p.bounds;
        const dims = g.d === 3 ? [0, 1, 2] : [0, 1];
        const now = dims.map((k) => b.max[k] - b.min[k]);
        const given = dims.map((k) => (size[k] > 0 && now[k] > 0 ? size[k] / now[k] : null));
        const any = given.find((f) => f !== null) ?? 1;
        const f = dims.map((k) => (given[k] !== null ? given[k] : auto[k] ? any : 1));
        return g.d === 3 ? g3(g.s.scale(f[0], f[1], f[2]), g.color) : g2(g.p.scale(f[0], f[1]), g.color);
      });
    },
    mirror(a, groups) {
      const { v = [1, 0, 0] } = named(a, ["v"]);
      const [x, y, z] = vecArg(v, 3, 0, "mirror");
      if (!x && !y && !z) return groups.flat();
      return groups.flat().map((g) => (g.d === 3 ? g3(g.s.mirror(x, y, z), g.color) : g2(g.p.mirror(x, y), g.color)));
    },
    multmatrix(a, groups) {
      const { m } = named(a, ["m"]);
      if (!isList(m)) fail("multmatrix needs a matrix m");
      const rows = transformMatrix(m);
      return groups.flat().map((g) => applyMatrix(g, rows));
    },
    color(a, groups) {
      const o = named(a, ["c", "alpha"]);
      let c = o.c;
      if (isList(c)) c = "#" + c.slice(0, 3).map((x) => Math.round(Math.max(0, Math.min(1, x)) * 255).toString(16).padStart(2, "0")).join("");
      return groups.flat().map((g) => ({ ...g, color: typeof c === "string" ? c : g.color }));
    },
    offset(a, groups, env) {
      const o = named(a, ["r"]);
      return merge(groups.flat()).map((g) => {
        if (g.d !== 2) fail("offset() works on 2D shapes");
        if (isNum(o.r)) return g2(g.p.offset(o.r, { join: "round" }), g.color);
        const d = isNum(o.delta) ? o.delta : 1;
        return g2(g.p.offset(d, { join: truthy(o.chamfer) ? "square" : "miter" }), g.color);
      });
    },
    hull(a, groups) {
      return merge(groups.flat()).map((g) => (g.d === 3 ? g3(g.s.hull(), g.color) : g2(g.p.hull(), g.color)));
    },
    minkowski(a, groups) {
      const all = groups.flat();
      const s = all.filter((g) => g.d === 3);
      const p = all.filter((g) => g.d === 2);
      const out = [];
      if (s.length) out.push(g3(s.slice(1).reduce((acc, g) => new Solid(__n.sbool("minkowski", acc.h, g.s.h)), s[0].s), s[0].color));
      if (p.length) {
        // 2D: summing with a circle is an exact round offset.
        let acc = p[0];
        for (const g of p.slice(1)) {
          if (g.circle) acc = g2(acc.p.offset(g.circle, { join: "round" }), acc.color);
          else if (acc.circle) acc = g2(g.p.offset(acc.circle, { join: "round" }), g.color);
          else fail("minkowski() of 2D shapes: Forma sums with a circle only (it is an offset)");
        }
        out.push(acc);
      }
      return out;
    },
    linear_extrude(a, groups) {
      const o = named(a, ["height", "v", "center", "convexity", "twist", "slices", "scale"]);
      const h = isNum(o.height) ? o.height : isList(o.v) ? Math.hypot(...o.v) : 100;
      const twist = isNum(o.twist) ? o.twist : 0;
      const scaleTop = isNum(o.scale) ? o.scale : isList(o.scale) ? vecArg(o.scale, 2, 1, "scale") : 1;
      return merge(groups.flat())
        .filter((g) => g.d === 2)
        .map((g) => {
          // OpenSCAD twists clockwise (seen from above) for a positive angle.
          let s = g.p.extrude(h, { twist: -twist, slices: isNum(o.slices) ? o.slices : 0, scaleTop });
          if (truthy(o.center)) s = s.translate(0, 0, -h / 2);
          return g3(s, g.color);
        });
    },
    rotate_extrude(a, groups) {
      const o = named(a, ["angle", "convexity"]);
      const ang = isNum(o.angle) ? Math.max(-360, Math.min(360, o.angle)) : 360;
      return merge(groups.flat())
        .filter((g) => g.d === 2)
        .map((g) => {
          const b = g.p.bounds;
          if (b.min[0] < -1e-9 && b.max[0] > 1e-9) fail("rotate_extrude(): the shape must lie on one side of the axis (all x ≥ 0)");
          const p = b.max[0] <= 1e-9 ? g.p.mirror(1, 0) : g.p;
          return g3(ang < 0 ? p.revolve(-ang).mirror(0, 1, 0) : p.revolve(ang), g.color);
        });
    },
  };

  /** Runs a list of statements: definitions first, then assignments, then geometry (one group each). */
  function runBlock(stmts, env) {
    for (const s of stmts) {
      if (s.k === "module") env.mods.set(s.name, { params: s.params, body: s.body, env });
      if (s.k === "function") env.funcs.set(s.name, new Func(s.params, s.body, env, s.name));
    }
    // The last assignment to a name wins, evaluated where the first one stood.
    const last = new Map();
    for (const s of stmts) if (s.k === "assign") last.set(s.name, s);
    const done = new Set();
    for (const s of stmts) {
      if (s.k !== "assign" || done.has(s.name)) continue;
      done.add(s.name);
      if (env.overrides && env.overrides.has(s.name)) {
        env.vars.set(s.name, env.overrides.get(s.name));
        continue;
      }
      env.vars.set(s.name, evalExpr(last.get(s.name).value, env));
    }
    const roots = stmts.filter((s) => s.root);
    const groups = [];
    for (const s of roots.length ? roots : stmts) {
      if (s.k === "module" || s.k === "function" || s.k === "assign") continue;
      groups.push(runStatement(s, env));
    }
    return groups;
  }

  function runStatement(s, env) {
    switch (s.k) {
      case "block":
        return runBlock(s.body, new Env(env)).flat();
      case "if": {
        const branch = truthy(evalExpr(s.cond, env)) ? s.then : s.other;
        return branch ? runBlock([branch], new Env(env)).flat() : [];
      }
      case "for": {
        const out = [];
        iterate(s.vars, env, (inner) => out.push(...runBlock([s.body], inner).flat()));
        return out;
      }
      case "ifor": {
        const each = [];
        iterate(s.vars, env, (inner) => each.push(runBlock([s.body], inner).flat()));
        return OPS.intersection([], each);
      }
      case "let": {
        const inner = new Env(env);
        for (const v of s.vars) inner.vars.set(v.name, v.value ? evalExpr(v.value, inner) : undefined);
        return runBlock([s.body], inner).flat();
      }
      case "call":
        return callModule(s, env);
    }
    return [];
  }

  function callModule(s, env) {
    const name = s.name;
    if (name === "echo") {
      messages.push(s.args.map((a) => (a.name ? `${a.name} = ` : "") + str(evalExpr(a.value, env))).join(", "));
      return runBlock(s.children, new Env(env)).flat();
    }
    if (name === "assert") {
      const [c, msg] = s.args.map((a) => evalExpr(a.value, env));
      if (!truthy(c)) fail(`assertion failed${msg !== undefined ? `: ${str(msg)}` : ""}`, s.line);
      return runBlock(s.children, new Env(env)).flat();
    }
    if (name === "children") {
      const ch = env.childrenOf;
      if (!ch) return [];
      const [idx] = s.args.map((a) => evalExpr(a.value, env));
      const chEnv = new Env(ch.env, env);
      const groups = runBlock(ch.stmts, chEnv);
      if (idx === undefined) return groups.flat();
      const pick = isNum(idx) ? [idx] : isList(idx) || isRange(idx) ? [...listOf(idx)] : [];
      return pick.flatMap((i) => groups[i] ?? []);
    }
    const user = env.mod(name);
    if (user) {
      if (++depth > 3000) fail(`${name}() calls itself too deeply (over 3000)`, s.line);
      try {
        const inner = bind(user.params, evalArgs(s.args, env), user.env, env, s.line);
        inner.childrenOf = { stmts: s.children, env };
        inner.vars.set("$children", countChildren(s.children, env));
        const body = user.body ? (user.body.k === "block" ? user.body.body : [user.body]) : [];
        return runBlock(body, inner).flat();
      } finally {
        depth--;
      }
    }
    const args = evalArgs(s.args, env);
    // $fn and friends given at a built-in call apply to it.
    const callEnv = new Env(env);
    for (const a of args) if (a.name && a.name.startsWith("$")) callEnv.vars.set(a.name, a.value);
    if (MODULES[name]) {
      if (s.children.length) runBlock(s.children, new Env(callEnv));
      return MODULES[name](args, callEnv);
    }
    if (OPS[name]) {
      const groups = runBlock(s.children, new Env(callEnv));
      return OPS[name](args, groups, callEnv);
    }
    fail(`unknown module ${name}()`, s.line);
  }

  function countChildren(stmts) {
    return stmts.filter((x) => x.k !== "assign" && x.k !== "module" && x.k !== "function").length;
  }

  /** Top-level number values, with customizer ranges and the comment above as description. */
  function parameters(src) {
    const { body, toks } = parse(src);
    const out = [];
    const seen = new Set();
    for (const s of body) {
      if (s.k !== "assign" || s.name.startsWith("$")) continue;
      let v = s.value;
      let sign = 1;
      if (v.k === "un" && v.op === "-" && v.a.k === "num") {
        sign = -1;
        v = v.a;
      }
      if (v.k !== "num" || typeof v.v !== "number") continue;
      if (seen.has(s.name)) continue;
      seen.add(s.name);
      // The comment on the same line after it, and on the line above.
      const same = toks.find((t) => t.t === "comment" && t.line === s.line);
      const above = toks.find((t) => t.t === "comment" && t.line === s.line - 1);
      const p = { name: s.name, value: sign * v.v, description: above ? above.v.trim() : "" };
      const range = same && /\[\s*(-?[\d.]+)\s*:\s*(-?[\d.]+)\s*(?::\s*(-?[\d.]+)\s*)?\]/.exec(same.v);
      if (range) {
        const nums = range.slice(1).filter((x) => x !== undefined).map(Number);
        p.min = nums[0];
        p.max = nums[nums.length - 1];
        if (nums.length === 3) p.step = nums[1];
      } else if (same && same.v.trim() && !p.description) p.description = same.v.trim();
      out.push(p);
    }
    return out;
  }

  function run(src, overrides = {}, options = {}) {
    messages.length = 0;
    depth = 0;
    const { body } = parse(src);
    const env = new Env(null);
    env.overrides = new Map(Object.entries(overrides ?? {}).filter(([, v]) => v !== undefined));
    const geoms = unite(runBlock(body, env).flat());
    const solids = geoms.filter((g) => g.d === 3);
    if (!solids.length) {
      if (geoms.some((g) => g.d === 2)) fail("the file makes only 2D shapes: linear_extrude() or rotate_extrude() them to make a part");
      fail("the file makes no geometry");
    }
    const colored = new Set(solids.map((g) => g.color ?? ""));
    if (colored.size <= 1) return solids[0].s;
    // Coloured pieces become parts, their colour as the material's look.
    const names = new Map();
    return solids.map((g) => {
      const base = g.color ? `${g.color[0].toUpperCase()}${g.color.slice(1)} part` : "Part";
      const n = (names.get(base) ?? 0) + 1;
      names.set(base, n);
      return part(n === 1 ? base : `${base} ${n}`, g.s, { material: options.material ?? g.color ?? undefined, process: options.process });
    });
  }

  return { run, parameters, messages, parse };
})();

/**
 * Builds an OpenSCAD source. `values` replaces top-level assignments by
 * name (the shape's parameters); `{ material, process }` for the parts.
 */
function openscad(source, values = {}, options = {}) {
  return __scad.run(source, values, options);
}
