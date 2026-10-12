// The bill of materials and sourcing workbench: what to make and buy, what it
// costs, and who quotes it. The engine works the BOM out (configurations,
// nested assemblies, merging, estimates, awards); this draws it.

import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatedNumber } from "./AnimatedNumber";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { PROCESS_NAMES, assemblyDef, instance, materialLook, program } from "./model";
import { BuildingView } from "./Building";
import { SourcingView } from "./Sourcing";

export type Estimate = { material?: number; process?: number; setup: number; hours?: number; basis: string[] };
export type AwardView = { id: string; supplier: string; supplier_name: string; quote?: string; unit_price: number | null; currency: string; lead_days?: number };
export type Stock = {
  standard?: string;
  part_number: string;
  description?: string;
  manufacturer?: string;
  mpn?: string;
  unit_price?: number;
  currency?: string;
  pack?: number;
  mass_g?: number;
  sources?: { supplier: string; sku?: string; url?: string; unit_price?: number; currency?: string }[];
};
export type MakeLine = {
  key: string;
  item: number;
  name: string;
  shape: string;
  config?: string;
  part?: string;
  material?: string;
  material_id?: string;
  process?: string;
  qty: number;
  volume_cm3: number;
  mass_kg: number | null;
  size_mm: [number, number, number];
  cut_mm?: number;
  files: string[];
  estimate: Estimate;
  award?: AwardView;
  unit_cost: number | null;
  extended: number | null;
  cost_source: "award" | "estimate" | "none";
  used_in: string[];
};
export type BuyLine = {
  key: string;
  item: number;
  origin: string;
  name: string;
  shape?: string;
  part?: string;
  stock: Stock | null;
  qty: number;
  mass_kg: number | null;
  order_qty: number;
  unit_price: number | null;
  extended: number | null;
  award?: AwardView;
  cost_source: "award" | "list" | "none";
  used_in: string[];
};
export type BomNode = {
  name: string;
  kind: "assembly" | "shape" | "part" | "item";
  qty: number;
  total: number;
  line?: string;
  class?: "make" | "buy";
  id?: string;
  config?: string;
  instances?: string[];
  children?: BomNode[];
};
export type Flag = { line?: string; level: "warning" | "info"; text: string };
export type Break = { qty: number; make: number; buy: number; per_unit: number; total: number; complete: boolean };
export type Bom = {
  name: string;
  root?: string;
  config?: string;
  make: MakeLine[];
  buy: BuyLine[];
  tree: BomNode[];
  flags: Flag[];
  totals: { currency: string; quantity: number; make: number; buy: number; per_unit: number; total: number; unpriced: number; mass_kg: number | null; breaks: Break[] };
};

export type Run = (operation: string, parameters: Record<string, unknown>) => Promise<{ output: Record<string, unknown> } | null | unknown>;

/** Money in a currency, compact for small amounts ($0.042) and grouped for large. */
export function money(v: number | null | undefined, currency = "USD", digits?: number) {
  if (v == null || !Number.isFinite(v)) return "—";
  const d = digits ?? (Math.abs(v) < 1 && v !== 0 ? 3 : 2);
  try {
    return new Intl.NumberFormat("en-US", { style: "currency", currency, minimumFractionDigits: d, maximumFractionDigits: d }).format(v);
  } catch {
    return `${v.toFixed(d)} ${currency}`;
  }
}

const kg = (v: number | null | undefined) => (v == null ? "—" : v >= 1 ? `${v.toLocaleString("en-US", { maximumFractionDigits: 2 })} kg` : `${(v * 1000).toLocaleString("en-US", { maximumFractionDigits: 1 })} g`);
const dims = (s: [number, number, number]) => s.map((v) => (v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2))).join(" × ");

export const processName = (p?: string | null) => (p ? (PROCESS_NAMES[p] ?? p) : "No process");

const SOURCE_LABEL: Record<string, string> = { award: "Quoted", estimate: "Estimate", list: "List", none: "No price" };

/** One fetch of the BOM, re-fetched (quietly, keeping what shows) as the design changes. */
export function useBom(scope: string, qty: number, config: string | null, revision: number) {
  const [bom, setBom] = useState<Bom | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  useEffect(() => {
    const n = ++seq.current;
    setLoading(true);
    const t = window.setTimeout(
      () => {
        const q = new URLSearchParams({ qty: String(qty) });
        if (config) q.set("config", config);
        fetch(projectUrl(`/bom/${scope}?${q}`))
          .then(async (r) => {
            if (!r.ok) throw new Error(await r.text());
            return r.json() as Promise<Bom>;
          })
          .then((b) => {
            if (n !== seq.current) return;
            setBom(b);
            setError(null);
          })
          .catch((e: Error) => n === seq.current && setError(e.message))
          .finally(() => n === seq.current && setLoading(false));
      },
      bom ? 250 : 0,
    );
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, qty, config, revision]);
  return { bom, error, loading };
}

export type Bench = "bom" | "sourcing" | "building";

/** What a BOM can be of: the selected item when it is a shape, instance or assembly, else the project. */
function scopeOf(entities: ReadonlyMap<string, EntityView>, id: string | null): { id: string; name: string } | null {
  const e = id ? entities.get(id) : undefined;
  if (!e) return null;
  const inst = instance(e);
  if (inst) return { id: e.id, name: inst.name };
  const p = program(e);
  if (p) return { id: e.id, name: p.name };
  const a = assemblyDef(e);
  if (a) return { id: e.id, name: a.name };
  return null;
}

export function Workbench(props: {
  tab: Bench;
  onTab: (t: Bench) => void;
  onClose: () => void;
  entities: ReadonlyMap<string, EntityView>;
  revision: number;
  selected: string | null;
  run: Run;
  notify: (m: string) => void;
  onSelect: (id: string) => void;
}) {
  const [rfqOpen, setRfqOpen] = useState<string | null>(null);
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && !(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) && props.onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [props]);
  return (
    <section className="bench floating" aria-label="Bill of materials and sourcing">
      <header className="bench-head">
        <div className="bench-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={props.tab === "bom"} className={props.tab === "bom" ? "active" : ""} onClick={() => props.onTab("bom")}>
            <Icon name="list" /> Bill of materials
          </button>
          <button type="button" role="tab" aria-selected={props.tab === "sourcing"} className={props.tab === "sourcing" ? "active" : ""} onClick={() => props.onTab("sourcing")}>
            <Icon name="truck" /> Sourcing
          </button>
          <button type="button" role="tab" aria-selected={props.tab === "building"} className={props.tab === "building" ? "active" : ""} onClick={() => props.onTab("building")}>
            <Icon name="building" /> Building
          </button>
        </div>
        <IconButton icon="close" label="Close (Esc)" onClick={props.onClose} />
      </header>
      {props.tab === "bom" ? (
        <BomView
          {...props}
          onRequest={(id) => {
            setRfqOpen(id);
            props.onTab("sourcing");
          }}
        />
      ) : props.tab === "building" ? (
        <BuildingView entities={props.entities} revision={props.revision} selected={props.selected} />
      ) : (
        <SourcingView entities={props.entities} revision={props.revision} run={props.run} notify={props.notify} open={rfqOpen} onOpen={setRfqOpen} onBom={() => props.onTab("bom")} />
      )}
    </section>
  );
}

function BomView(props: {
  entities: ReadonlyMap<string, EntityView>;
  revision: number;
  selected: string | null;
  run: Run;
  notify: (m: string) => void;
  onSelect: (id: string) => void;
  onRequest: (rfq: string) => void;
}) {
  const picked = scopeOf(props.entities, props.selected);
  const [useSelection, setUseSelection] = useState(true);
  const scope = useSelection && picked ? picked : null;
  const [qty, setQty] = useState(1);
  const [qtyText, setQtyText] = useState("1");
  const [tab, setTab] = useState<"make" | "buy" | "tree">("make");
  const [open, setOpen] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [flagsOpen, setFlagsOpen] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  const { bom, error, loading } = useBom(scope?.id ?? "project", qty, null, props.revision);
  const cur = bom?.totals.currency ?? "USD";

  const setQuantity = (n: number) => {
    const v = Math.max(1, Math.min(1_000_000, Math.round(n) || 1));
    setQty(v);
    setQtyText(String(v));
  };

  // Lines chosen for a request for quote drop out when they leave the BOM.
  useEffect(() => {
    if (!bom) return;
    const keys = new Set([...bom.make.map((l) => l.key), ...bom.buy.map((l) => l.key)]);
    setChosen((c) => new Set([...c].filter((k) => keys.has(k))));
  }, [bom]);

  const goTo = (line: string) => {
    if (!bom) return;
    setTab(bom.make.some((l) => l.key === line) ? "make" : "buy");
    setOpen(line);
    setFlash(line);
    window.setTimeout(() => document.getElementById(`bom-${line}`)?.scrollIntoView({ block: "center", behavior: "smooth" }), 30);
    window.setTimeout(() => setFlash(null), 1600);
  };

  const toggle = (key: string) =>
    setChosen((c) => {
      const n = new Set(c);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const suppliers = useMemo(() => [...props.entities.values()].filter((e) => e.components["sourcing.supplier"]), [props.entities]);

  const requestQuotes = async () => {
    if (!bom || chosen.size === 0) return;
    const lines = [...bom.make, ...bom.buy].filter((l) => chosen.has(l.key)).map((l) => ({ line: l.key, name: l.name, qty: l.qty }));
    const processes = new Set(bom.make.filter((l) => chosen.has(l.key)).map((l) => l.process ?? ""));
    const bought = bom.buy.some((l) => chosen.has(l.key));
    // Suppliers whose kind covers at least one chosen line are asked by default.
    const matching = suppliers
      .filter((s) => {
        const kind = ((s.components["sourcing.supplier"] as { kind?: string[] }).kind ?? []) as string[];
        return kind.length === 0 || kind.some((k) => processes.has(k) || (k === "distributor" && bought));
      })
      .map((s) => s.id);
    const name = `${bom.name} — ${lines.length === 1 ? lines[0].name : `${lines.length} lines`}`;
    const r = (await props.run("sourcing.rfq.create", {
      name,
      scope: bom.root ?? null,
      config: bom.config ?? null,
      lines,
      quantities: [...new Set([1, 10, 100, qty])].sort((a, b) => a - b),
      suppliers: matching,
    })) as { output?: { id?: string } } | null;
    const id = r?.output?.id;
    if (id) {
      setChosen(new Set());
      props.notify(`Request for quote made with ${lines.length} line${lines.length === 1 ? "" : "s"}${matching.length ? ` for ${matching.length} supplier${matching.length === 1 ? "" : "s"}` : ""}`);
      props.onRequest(id);
    }
  };

  const exportUrl = (format: string, view = "flat") => projectUrl(`/bom/${scope?.id ?? "project"}?qty=${qty}&format=${format}&view=${view}`);
  const warnings = bom?.flags.filter((f) => f.level === "warning").length ?? 0;
  const flagged = useMemo(() => new Set(bom?.flags.filter((f) => f.line && f.level === "warning").map((f) => f.line!) ?? []), [bom]);
  const currentBreak = bom?.totals.breaks.find((b) => b.qty === qty);

  return (
    <div className="bom">
      <div className="bom-bar">
        <div className="bom-scope">
          <button type="button" className={`scope-chip ${!scope ? "on" : ""}`} onClick={() => setUseSelection(false)}>
            <Icon name="layers" /> Whole project
          </button>
          {picked && (
            <button type="button" className={`scope-chip ${scope ? "on" : ""}`} onClick={() => setUseSelection(true)} title="The item selected in the view">
              <Icon name="cube" /> {picked.name}
            </button>
          )}
          {!picked && <span className="bom-hint">Select a part or assembly to list just that</span>}
        </div>
        <div className="bom-actions">
          <label className="qty-field" title="Finished units to build">
            <span>Build</span>
            <button type="button" aria-label="Fewer" onClick={() => setQuantity(qty > 10 ? Math.round(qty / 10) : qty - 1)}>
              <Icon name="minus" />
            </button>
            <input
              inputMode="numeric"
              value={qtyText}
              aria-label="Units to build"
              onChange={(e) => setQtyText(e.target.value.replace(/[^0-9]/g, ""))}
              onBlur={() => setQuantity(Number(qtyText))}
              onKeyDown={(e) => e.key === "Enter" && setQuantity(Number(qtyText))}
            />
            <button type="button" aria-label="More" onClick={() => setQuantity(qty >= 10 ? qty * 10 : qty + 1)}>
              <Icon name="plus" />
            </button>
            <span>{qty === 1 ? "unit" : "units"}</span>
          </label>
          <ExportMenu
            items={[
              { label: "BOM (CSV)", hint: "Every line, Make then Buy", href: exportUrl("csv") },
              { label: "Indented BOM (CSV)", hint: "The assembly tree with quantities", href: exportUrl("csv", "indented") },
              { label: "BOM (JSON)", hint: "Everything, for other tools", href: exportUrl("json"), download: `${bom?.name ?? "bom"}.json` },
              { label: "Fabrication package", hint: "Every part's files and a cut list", href: projectUrl(`/export/${scope?.id ?? "project"}.zip`) },
            ]}
          />
        </div>
      </div>

      {error && !bom && <div className="bom-empty">{error}</div>}
      {!bom && !error && <BomSkeleton />}
      {bom && (
        <>
          <div className={`bom-summary ${loading ? "refreshing" : ""}`}>
            <div className="kpi lead">
              <small>Cost per unit{bom.totals.unpriced > 0 ? " · partial" : ""}</small>
              <strong>
                <AnimatedNumber value={bom.totals.per_unit} format={(v) => money(v, cur, 2)} />
              </strong>
              <span>
                {qty > 1 ? `${money(bom.totals.total, cur, 0)} for ${qty.toLocaleString()} units` : "estimates unless quoted"}
              </span>
            </div>
            <div className="kpi">
              <small>Make</small>
              <strong>
                <AnimatedNumber value={bom.totals.make / qty} format={(v) => money(v, cur, 2)} />
              </strong>
              <span>{bom.make.reduce((n, l) => n + l.qty, 0)} parts · {bom.make.length} lines</span>
            </div>
            <div className="kpi">
              <small>Buy</small>
              <strong>
                <AnimatedNumber value={bom.totals.buy / qty} format={(v) => money(v, cur, 2)} />
              </strong>
              <span>{bom.buy.reduce((n, l) => n + l.qty, 0)} items · {bom.buy.length} lines</span>
            </div>
            <div className="kpi">
              <small>Mass</small>
              <strong>{kg(bom.totals.mass_kg)}</strong>
              <span>{bom.totals.mass_kg == null ? "some parts have no material" : "per unit"}</span>
            </div>
            <div className="breaks" role="group" aria-label="Cost per unit by production quantity">
              {bom.totals.breaks.map((b) => (
                <button type="button" key={b.qty} className={b.qty === qty ? "on" : ""} onClick={() => setQuantity(b.qty)} title={`${money(b.total, cur, 0)} for ${b.qty.toLocaleString()}${b.complete ? "" : " · some lines have no price"}`}>
                  <small>×{b.qty.toLocaleString()}</small>
                  <strong>{money(b.per_unit, cur, 2)}</strong>
                  <i style={{ width: `${Math.max(6, (b.per_unit / Math.max(...bom.totals.breaks.map((x) => x.per_unit), 1e-9)) * 100)}%` }} />
                </button>
              ))}
            </div>
          </div>

          {bom.flags.length > 0 && (
            <div className={`bom-flags ${warnings ? "warn" : ""}`}>
              <button type="button" className="flags-head" onClick={() => setFlagsOpen((o) => !o)} aria-expanded={flagsOpen}>
                <Icon name="alert" />
                <span>
                  {warnings > 0 ? `${warnings} to fix` : ""}
                  {warnings > 0 && bom.flags.length > warnings ? " · " : ""}
                  {bom.flags.length > warnings ? `${bom.flags.length - warnings} note${bom.flags.length - warnings === 1 ? "" : "s"}` : ""}
                </span>
                <em>{bom.flags.find((f) => f.level === "warning")?.text ?? bom.flags[0].text}</em>
                <Icon name="down" />
              </button>
              {flagsOpen && (
                <ul>
                  {bom.flags.map((f, i) => (
                    <li key={i} className={f.level}>
                      <span className="dot" />
                      {f.line ? (
                        <button type="button" onClick={() => goTo(f.line!)}>
                          {f.text}
                        </button>
                      ) : (
                        <span>{f.text}</span>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}

          <div className="bom-tabs">
            <div className="segmented">
              <button type="button" className={tab === "make" ? "active" : ""} onClick={() => setTab("make")}>
                Make <b>{bom.make.length}</b>
              </button>
              <button type="button" className={tab === "buy" ? "active" : ""} onClick={() => setTab("buy")}>
                Buy <b>{bom.buy.length}</b>
              </button>
              <button type="button" className={tab === "tree" ? "active" : ""} onClick={() => setTab("tree")}>
                Indented
              </button>
            </div>
            <span className="bom-note">
              {tab === "tree" ? "Quantities per parent · total per unit" : `Quantities per unit${qty > 1 ? ` · extended for ${qty.toLocaleString()}` : ""}`}
              {currentBreak && !currentBreak.complete ? " · some lines have no price" : ""}
            </span>
          </div>

          <div className="bom-table-wrap">
            {tab === "make" && (
              <MakeTable bom={bom} qty={qty} open={open} onOpen={setOpen} chosen={chosen} onToggle={toggle} flash={flash} flagged={flagged} scope={scope?.id ?? "project"} onSelect={props.onSelect} entities={props.entities} />
            )}
            {tab === "buy" && <BuyTable bom={bom} qty={qty} open={open} onOpen={setOpen} chosen={chosen} onToggle={toggle} flash={flash} flagged={flagged} />}
            {tab === "tree" && <TreeTable bom={bom} onLine={goTo} />}
            {tab !== "tree" && (tab === "make" ? bom.make : bom.buy).length === 0 && (
              <div className="bom-empty">{tab === "make" ? "Nothing to make here." : "Nothing bought here. Programs declare bought items with stock({ partNumber, … })."}</div>
            )}
          </div>

          <div className={`bom-dock ${chosen.size ? "shown" : ""}`} aria-hidden={!chosen.size}>
            <span>
              <b>{chosen.size}</b> line{chosen.size === 1 ? "" : "s"} chosen
            </span>
            <button type="button" className="secondary" onClick={() => setChosen(new Set())}>
              Clear
            </button>
            <button type="button" className="primary" onClick={requestQuotes}>
              <Icon name="mail" /> Request quotes
            </button>
          </div>
        </>
      )}
    </div>
  );
}

function BomSkeleton() {
  return (
    <div className="bom-skeleton" aria-busy="true" aria-label="Working out the bill of materials">
      <div className="sk-row">
        {[0, 1, 2, 3].map((i) => (
          <i key={i} className="sk kpi-sk" />
        ))}
      </div>
      {[0, 1, 2, 3, 4, 5].map((i) => (
        <i key={i} className="sk line-sk" style={{ animationDelay: `${i * 60}ms` }} />
      ))}
    </div>
  );
}

function ExportMenu({ items }: { items: { label: string; hint: string; href: string; download?: string }[] }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", away);
    return () => window.removeEventListener("mousedown", away);
  }, [open]);
  return (
    <div className="menu-host" ref={ref}>
      <button type="button" className="secondary" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Icon name="download" /> Export <Icon name="down" />
      </button>
      {open && (
        <div className="menu floating" role="menu">
          {items.map((i) => (
            <a key={i.label} role="menuitem" href={i.href} download={i.download ?? ""} onClick={() => setOpen(false)}>
              <strong>{i.label}</strong>
              <small>{i.hint}</small>
            </a>
          ))}
        </div>
      )}
    </div>
  );
}

function SourceBadge({ source, award }: { source: string; award?: AwardView }) {
  return (
    <span className={`src-badge ${source === "award" ? "quoted" : source}`} title={award ? `${award.supplier_name}${award.lead_days ? ` · ${award.lead_days} days` : ""}` : undefined}>
      {source === "award" && award ? award.supplier_name : SOURCE_LABEL[source]}
    </span>
  );
}

function Check({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      className={`row-check ${on ? "on" : ""}`}
      role="checkbox"
      aria-checked={on}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      {on && <Icon name="check" />}
    </button>
  );
}

function MakeTable(props: {
  bom: Bom;
  qty: number;
  open: string | null;
  onOpen: (k: string | null) => void;
  chosen: Set<string>;
  onToggle: (k: string) => void;
  flash: string | null;
  flagged: Set<string>;
  scope: string;
  onSelect: (id: string) => void;
  entities: ReadonlyMap<string, EntityView>;
}) {
  const { bom, qty } = props;
  const cur = bom.totals.currency;
  const all = bom.make.length > 0 && bom.make.every((l) => props.chosen.has(l.key));
  const fileUrl = (l: MakeLine, ext: string) => projectUrl(`/bom/${props.scope}/file?line=${encodeURIComponent(l.key)}&format=${ext}&qty=${qty}`);
  return (
    <table className="bom-table">
      <thead>
        <tr>
          <th className="c-check">
            <Check on={all} label="Choose every line" onClick={() => bom.make.forEach((l) => (all ? props.chosen.has(l.key) : !props.chosen.has(l.key)) && props.onToggle(l.key))} />
          </th>
          <th className="c-item">#</th>
          <th>Part</th>
          <th>Material</th>
          <th>Process</th>
          <th className="num">Qty</th>
          <th className="num">Size mm</th>
          <th className="num">Mass</th>
          <th className="num">Each</th>
          <th className="num">Extended</th>
          <th>Files</th>
        </tr>
      </thead>
      <tbody>
        {bom.make.map((l) => {
          const isOpen = props.open === l.key;
          return (
            <Fragment key={l.key}>
              <tr id={`bom-${l.key}`} className={`${isOpen ? "open" : ""} ${props.flash === l.key ? "flash" : ""} ${props.chosen.has(l.key) ? "chosen" : ""}`} onClick={() => props.onOpen(isOpen ? null : l.key)}>
                <td className="c-check">
                  <Check on={props.chosen.has(l.key)} label={`Choose ${l.name}`} onClick={() => props.onToggle(l.key)} />
                </td>
                <td className="c-item">{l.item}</td>
                <td className="c-name">
                  <strong>
                    {props.flagged.has(l.key) && <span className="warn-dot" title="Needs attention" />}
                    {l.name}
                  </strong>
                  <small>{l.used_in.join(" · ")}</small>
                </td>
                <td>
                  {l.material ? (
                    <span className="mat">
                      <i style={{ background: materialLook(l.material).color }} />
                      {l.material}
                    </span>
                  ) : (
                    <span className="none">No material</span>
                  )}
                </td>
                <td>
                  <span className={`proc ${l.process ?? "none"}`}>{processName(l.process)}</span>
                </td>
                <td className="num qty">
                  {l.qty.toLocaleString()}
                  {qty > 1 && <small>{(l.qty * qty).toLocaleString()}</small>}
                </td>
                <td className="num dim">{dims(l.size_mm)}</td>
                <td className="num">{kg(l.mass_kg)}</td>
                <td className="num cost">
                  {money(l.unit_cost, cur)}
                  <SourceBadge source={l.cost_source} award={l.award} />
                </td>
                <td className="num strong">{money(l.extended, cur, 2)}</td>
                <td className="files" onClick={(e) => e.stopPropagation()}>
                  {l.files.map((f) => (
                    <a key={f} href={fileUrl(l, f)} download className="file-chip" title={`Download ${f.toUpperCase()}`}>
                      {f.toUpperCase()}
                    </a>
                  ))}
                </td>
              </tr>
              {isOpen && (
                <tr className="detail">
                  <td />
                  <td colSpan={10}>
                    <div className="line-detail">
                      <div>
                        <h4>{l.cost_source === "award" ? "Quoted price" : "How the estimate is made"}</h4>
                        {l.award && (
                          <p className="award-line">
                            <Icon name="check" /> Awarded to <b>{l.award.supplier_name}</b> at {money(l.award.unit_price, l.award.currency)} each
                            {l.award.lead_days ? ` · ${l.award.lead_days} days` : ""}
                          </p>
                        )}
                        <ul className="basis">
                          {l.estimate.basis.map((b, i) => (
                            <li key={i}>{b}</li>
                          ))}
                          {l.estimate.basis.length === 0 && <li>No estimate: give the part a process and its material a price.</li>}
                        </ul>
                        <div className="est-split">
                          <span>
                            Material <b>{money(l.estimate.material, cur)}</b>
                          </span>
                          <span>
                            Process <b>{money(l.estimate.process, cur)}</b>
                          </span>
                          <span>
                            Setup <b>{money(l.estimate.setup, cur, 2)}</b> / batch
                          </span>
                          {l.estimate.hours != null && (
                            <span>
                              Time <b>{l.estimate.hours < 1 ? `${(l.estimate.hours * 60).toFixed(1)} min` : `${l.estimate.hours.toFixed(2)} h`}</b>
                            </span>
                          )}
                        </div>
                        <p className="muted">Estimates are rough: material at its price, machine time at the shop rate. A quote replaces them.</p>
                      </div>
                      <div>
                        <h4>Part</h4>
                        <dl>
                          <dt>Volume</dt>
                          <dd>{l.volume_cm3.toLocaleString("en-US", { maximumFractionDigits: 2 })} cm³</dd>
                          {l.cut_mm != null && (
                            <>
                              <dt>Cut length</dt>
                              <dd>{(l.cut_mm / 1000).toLocaleString("en-US", { maximumFractionDigits: 2 })} m</dd>
                            </>
                          )}
                          {l.config && (
                            <>
                              <dt>Configuration</dt>
                              <dd>{l.config}</dd>
                            </>
                          )}
                          <dt>Used in</dt>
                          <dd>{l.used_in.join(", ")}</dd>
                        </dl>
                        {props.entities.get(l.shape) && (
                          <button type="button" className="secondary" onClick={() => props.onSelect(l.shape)}>
                            <Icon name="cursor" /> Show in the view
                          </button>
                        )}
                      </div>
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
      {bom.make.length > 0 && (
        <tfoot>
          <tr>
            <td colSpan={5}>Make total</td>
            <td className="num">{bom.make.reduce((n, l) => n + l.qty, 0)}</td>
            <td colSpan={2} />
            <td />
            <td className="num strong">{money(bom.totals.make, cur, 2)}</td>
            <td />
          </tr>
        </tfoot>
      )}
    </table>
  );
}

function BuyTable(props: { bom: Bom; qty: number; open: string | null; onOpen: (k: string | null) => void; chosen: Set<string>; onToggle: (k: string) => void; flash: string | null; flagged: Set<string> }) {
  const { bom, qty } = props;
  const cur = bom.totals.currency;
  const all = bom.buy.length > 0 && bom.buy.every((l) => props.chosen.has(l.key));
  return (
    <table className="bom-table">
      <thead>
        <tr>
          <th className="c-check">
            <Check on={all} label="Choose every line" onClick={() => bom.buy.forEach((l) => (all ? props.chosen.has(l.key) : !props.chosen.has(l.key)) && props.onToggle(l.key))} />
          </th>
          <th className="c-item">#</th>
          <th>Part number</th>
          <th>Standard</th>
          <th>Manufacturer</th>
          <th className="num">Qty</th>
          <th className="num">Order</th>
          <th className="num">Each</th>
          <th className="num">Extended</th>
          <th>Sources</th>
        </tr>
      </thead>
      <tbody>
        {bom.buy.map((l) => {
          const s = l.stock;
          const isOpen = props.open === l.key;
          return (
            <Fragment key={l.key}>
              <tr id={`bom-${l.key}`} className={`${isOpen ? "open" : ""} ${props.flash === l.key ? "flash" : ""} ${props.chosen.has(l.key) ? "chosen" : ""} ${!s ? "missing" : ""}`} onClick={() => props.onOpen(isOpen ? null : l.key)}>
                <td className="c-check">
                  <Check on={props.chosen.has(l.key)} label={`Choose ${l.name}`} onClick={() => props.onToggle(l.key)} />
                </td>
                <td className="c-item">{l.item}</td>
                <td className="c-name">
                  {s ? (
                    <strong className="pn">{s.part_number}</strong>
                  ) : (
                    <strong className="pn missing">
                      <span className="warn-dot" /> No part number
                    </strong>
                  )}
                  <small>
                    {s?.description ?? l.name}
                    {l.origin !== "stock" ? ` · from ${l.origin}` : ""}
                  </small>
                </td>
                <td>{s?.standard ?? <span className="none">—</span>}</td>
                <td>{s?.manufacturer ? `${s.manufacturer}${s.mpn ? ` · ${s.mpn}` : ""}` : <span className="none">—</span>}</td>
                <td className="num qty">
                  {l.qty.toLocaleString()}
                  {qty > 1 && <small>{(l.qty * qty).toLocaleString()}</small>}
                </td>
                <td className="num">
                  {l.order_qty.toLocaleString()}
                  {s?.pack && s.pack > 1 && <small className="pack">packs of {s.pack}</small>}
                </td>
                <td className="num cost">
                  {money(l.unit_price, cur)}
                  <SourceBadge source={l.cost_source} award={l.award} />
                </td>
                <td className="num strong">{money(l.extended, cur, 2)}</td>
                <td className="sources" onClick={(e) => e.stopPropagation()}>
                  {(s?.sources ?? []).slice(0, 3).map((x, i) =>
                    x.url ? (
                      <a key={i} href={x.url} target="_blank" rel="noreferrer" className="file-chip src">
                        {x.supplier}
                      </a>
                    ) : (
                      <span key={i} className="file-chip src" title={x.sku ? `SKU ${x.sku}` : undefined}>
                        {x.supplier}
                      </span>
                    ),
                  )}
                </td>
              </tr>
              {isOpen && (
                <tr className="detail">
                  <td />
                  <td colSpan={9}>
                    <div className="line-detail">
                      <div>
                        <h4>Item</h4>
                        {!s && <p className="fix">Bought part without a part number — give it stock() in its program, or use a part from the stock library.</p>}
                        <dl>
                          <dt>Model name</dt>
                          <dd>{l.name}</dd>
                          {s?.mpn && (
                            <>
                              <dt>MPN</dt>
                              <dd>{s.mpn}</dd>
                            </>
                          )}
                          {s?.mass_g != null && (
                            <>
                              <dt>Mass</dt>
                              <dd>{s.mass_g} g each</dd>
                            </>
                          )}
                          <dt>Used in</dt>
                          <dd>{l.used_in.join(", ")}</dd>
                        </dl>
                      </div>
                      <div>
                        <h4>Prices</h4>
                        {l.award && (
                          <p className="award-line">
                            <Icon name="check" /> Awarded to <b>{l.award.supplier_name}</b> at {money(l.award.unit_price, l.award.currency)}
                          </p>
                        )}
                        <dl>
                          {s?.unit_price != null && (
                            <>
                              <dt>List</dt>
                              <dd>{money(s.unit_price, s.currency ?? cur)}</dd>
                            </>
                          )}
                          {(s?.sources ?? []).map((x, i) => (
                            <Fragment key={i}>
                              <dt>{x.supplier}</dt>
                              <dd>
                                {x.unit_price != null ? money(x.unit_price, x.currency ?? cur) : "—"}
                                {x.sku ? ` · ${x.sku}` : ""}
                              </dd>
                            </Fragment>
                          ))}
                        </dl>
                      </div>
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
      {bom.buy.length > 0 && (
        <tfoot>
          <tr>
            <td colSpan={5}>Buy total</td>
            <td className="num">{bom.buy.reduce((n, l) => n + l.qty, 0)}</td>
            <td colSpan={2} />
            <td className="num strong">{money(bom.totals.buy, cur, 2)}</td>
            <td />
          </tr>
        </tfoot>
      )}
    </table>
  );
}

function TreeTable({ bom, onLine }: { bom: Bom; onLine: (k: string) => void }) {
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const items = useMemo(() => {
    const m = new Map<string, number>();
    for (const l of bom.make) m.set(l.key, l.item);
    for (const l of bom.buy) m.set(l.key, l.item);
    return m;
  }, [bom]);
  const rows: { n: BomNode; depth: number; path: string; kids: boolean }[] = [];
  const walk = (nodes: BomNode[], depth: number, at: string) =>
    nodes.forEach((n, i) => {
      const path = `${at}/${i}`;
      const kids = (n.children?.length ?? 0) > 0;
      rows.push({ n, depth, path, kids });
      if (kids && !closed.has(path)) walk(n.children!, depth + 1, path);
    });
  walk(bom.tree, 0, "");
  const flip = useCallback(
    (p: string) =>
      setClosed((c) => {
        const n = new Set(c);
        if (n.has(p)) n.delete(p);
        else n.add(p);
        return n;
      }),
    [],
  );
  return (
    <table className="bom-table tree">
      <thead>
        <tr>
          <th>Item</th>
          <th className="num">Qty</th>
          <th className="num">Total</th>
          <th>Kind</th>
          <th className="num">Line</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ n, depth, path, kids }) => (
          <tr key={path} className={`depth-${Math.min(depth, 6)} ${n.kind}`}>
            <td className="tree-name">
              <span style={{ paddingLeft: depth * 18 }} className="tree-indent">
                {kids ? (
                  <button type="button" className={`twisty ${closed.has(path) ? "" : "open"}`} onClick={() => flip(path)} aria-label={closed.has(path) ? "Expand" : "Collapse"}>
                    <Icon name="chevron" />
                  </button>
                ) : (
                  <i className="twisty-space" />
                )}
                <strong>{n.name}</strong>
                {n.instances && n.instances.length > 0 && <small>{n.instances.length > 4 ? `${n.instances.slice(0, 4).join(", ")} +${n.instances.length - 4}` : n.instances.join(", ")}</small>}
              </span>
            </td>
            <td className="num qty">×{n.qty}</td>
            <td className="num">{n.total}</td>
            <td>{n.class ? <span className={`cls ${n.class}`}>{n.class === "make" ? "Make" : "Buy"}</span> : <span className="kind">{n.kind === "assembly" ? "Assembly" : n.kind === "shape" ? "Shape" : "Item"}</span>}</td>
            <td className="num">
              {n.line && items.has(n.line) ? (
                <button type="button" className="item-link" onClick={() => onLine(n.line!)}>
                  {items.get(n.line)}
                </button>
              ) : null}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
