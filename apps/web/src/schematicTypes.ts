// What GET /pcb/schematic/<id> returns, shared by the schematic view and its panels.

export type P = [number, number];
export type Box = [number, number, number, number];
export type Draw =
  | { t: "line"; p: P[]; w: number; ink: string }
  | { t: "poly"; p: P[]; w: number; ink: string; fill: "none" | "ink" | "body" }
  | { t: "circle"; c: P; r: number; w: number; ink: string; fill: "none" | "ink" | "body" }
  | { t: "text"; at: P; s: string; size: number; angle: number; anchor: "start" | "middle" | "end"; ink: string }
  | { t: "styled"; p: P[]; w: number; color: string; fill: string | null; dash: "" | "dash" | "dot" | "dash_dot"; closed: boolean }
  | { t: "rich"; at: P; s: string; size: number; angle: number; anchor: "start" | "middle" | "end"; color: string; bold: boolean; italic: boolean }
  | { t: "image"; at: P; size: P; mime: string; data: string };
export type Dir = "left" | "right" | "up" | "down";
export type PinV = { number: string; name: string; etype: string; at: P; dir: Dir; hidden: boolean; net: string | null; pads: string[] };
export type SymbolV = {
  id: string;
  sheet: string;
  symbol: string;
  reference: string;
  value: string;
  footprint: string;
  unit: number;
  units: number;
  at: P;
  rotation: number;
  mirror: boolean;
  mpn: string | null;
  manufacturer: string | null;
  lcsc: string | null;
  description: string;
  dnp: boolean;
  virtual_part: boolean;
  exclude_from_board: boolean;
  component: string | null;
  known: boolean;
  category: string;
  body: Box;
  bbox: Box;
  draw: Draw[];
  pins: PinV[];
  fields: Record<string, string>;
  /** On another instance of a shared sheet: the sheet it is drawn from. */
  instance: string | null;
};
export type WireV = { id: string; sheet: string; points: P[]; net: string | null; color: string | null; width: number | null };
export type LabelKind = "local" | "global" | "hierarchical" | "power" | "directive";
export type LabelV = { id: string; sheet: string; kind: LabelKind; text: string; at: P; rotation: number; net: string | null; draw: Draw[]; bbox: Box };
export type SheetBoxV = { sheet: string; name: string; at: P; size: P; pins: [string, P][] };
export type SheetV = { id: string; name: string; paper: string; size: P; parent: string | null; instance_of: string | null; page: string; portrait: boolean; frame: Draw[]; boxes: SheetBoxV[]; boxes_draw: Draw[] };
export type NetV = { name: string; pins: string[]; pads: string[]; labels: string[]; sheets: string[]; named: boolean; class: string | null; color: string | null };
export type Erc = { code: string; severity: "error" | "warning"; message: string; sheet: string; at: P; objects: string[]; refs: string[]; key: string; excluded: boolean };
export type TextV = { id: string; sheet: string; at: P; text: string; source: string; size: number; justify: string; angle: number; bold: boolean; italic: boolean; color: string | null };
export type BusV = { id: string; sheet: string; points: P[]; name: string | null; members: { member: string; net: string | null }[] };
export type BusEntryV = { id: string; sheet: string; at: P; end: P };
export type GraphicV = { id: string; sheet: string; kind: "shape" | "text_box" | "table" | "image"; draw: Draw[]; bbox: Box; props: Record<string, unknown> };
export type NetClassV = { name: string; color?: string; wire_width?: number; track_width?: number; clearance?: number; via_diameter?: number; via_drill?: number; patterns?: string[] };
export type SheetItemV = { kind: "line" | "rect" | "text"; at: P; end?: P; corner?: string; text?: string; size?: number; bold?: boolean; width?: number };
export type SetupV = {
  bus_aliases: Record<string, string[]>;
  netclasses: NetClassV[];
  net_colors: Record<string, string>;
  erc: { severities?: Record<string, string>; pin_matrix?: Record<string, string>; exclusions?: { key: string; comment?: string }[] };
  field_templates: { name: string; value?: string; visible?: boolean }[];
  text_vars: Record<string, string>;
  drawing_sheet: { no_title_block?: boolean; no_zones?: boolean; margin?: number; items?: SheetItemV[] };
  pin_types: string[];
  pin_matrix: Record<string, string>;
};
export type TitleV = { title: string; company: string; revision: string; date: string; comments?: string[] };
export type SchematicV = {
  id: string;
  name: string;
  board: string | null;
  title: TitleV;
  sheets: SheetV[];
  symbols: SymbolV[];
  wires: WireV[];
  labels: LabelV[];
  junctions: { id: string | null; sheet: string; at: P }[];
  no_connects: { id: string; sheet: string; at: P }[];
  texts: TextV[];
  buses: BusV[];
  bus_entries: BusEntryV[];
  graphics: GraphicV[];
  nets: NetV[];
  violations: Erc[];
  stats: { sheets: number; symbols: number; parts: number; wires: number; labels: number; nets: number; errors: number; warnings: number };
  setup: SetupV;
};
export type Commit = (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>;
export type SelKind = "symbol" | "wire" | "label" | "junction" | "nc" | "text" | "bus" | "entry" | "graphic";
export type Sel = { kind: SelKind; id: string } | null;

/** Every selectable item on a sheet with its box (for box select, align, copy). */
export function itemsOnSheet(v: SchematicV, sheet: string): { kind: SelKind; id: string; box: Box }[] {
  const out: { kind: SelKind; id: string; box: Box }[] = [];
  const ptsBox = (p: P[]): Box => p.reduce<Box>((b, q) => [Math.min(b[0], q[0]), Math.min(b[1], q[1]), Math.max(b[2], q[0]), Math.max(b[3], q[1])], [Infinity, Infinity, -Infinity, -Infinity]);
  for (const s of v.symbols) if (s.sheet === sheet) out.push({ kind: "symbol", id: s.id, box: s.body });
  for (const w of v.wires) if (w.sheet === sheet) out.push({ kind: "wire", id: w.id, box: ptsBox(w.points) });
  for (const w of v.buses) if (w.sheet === sheet) out.push({ kind: "bus", id: w.id, box: ptsBox(w.points) });
  for (const e of v.bus_entries) if (e.sheet === sheet) out.push({ kind: "entry", id: e.id, box: ptsBox([e.at, e.end]) });
  for (const l of v.labels) if (l.sheet === sheet) out.push({ kind: "label", id: l.id, box: l.bbox });
  for (const j of v.junctions) if (j.sheet === sheet && j.id) out.push({ kind: "junction", id: j.id, box: [j.at[0], j.at[1], j.at[0], j.at[1]] });
  for (const n of v.no_connects) if (n.sheet === sheet) out.push({ kind: "nc", id: n.id, box: [n.at[0], n.at[1], n.at[0], n.at[1]] });
  for (const t of v.texts) if (t.sheet === sheet) out.push({ kind: "text", id: t.id, box: [t.at[0], t.at[1] - t.size / 2, t.at[0] + t.text.length * t.size * 0.6, t.at[1] + t.size / 2] });
  for (const g of v.graphics) if (g.sheet === sheet) out.push({ kind: "graphic", id: g.id, box: g.bbox });
  return out;
}

/** "D[0..7]", "{SDA SCL}", "I2C{SDA SCL}" or a bus alias. */
export function isBusName(text: string, aliases: Record<string, string[]>): boolean {
  const t = text.trim();
  if (/^[^\s[\]{}]+\[\d+\.\.\d+\]$/.test(t)) return true;
  if (/^[^\s{}]*\{.+\}$/.test(t)) return true;
  return Object.keys(aliases).some((a) => a.toLowerCase() === t.toLowerCase());
}
