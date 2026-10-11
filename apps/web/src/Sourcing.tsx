// Sourcing: suppliers, requests for quote and the quotes that answer them.
// Packages and email drafts are made here for the person to send; nothing
// leaves this computer on its own.

import { useEffect, useMemo, useState } from "react";
import { type Run, money, processName } from "./Bom";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";

type PriceBreak = { qty: number; unit_price: number };
export type Supplier = { name: string; kind: string[]; website?: string; email?: string; phone?: string | null; country?: string; currency: string; lead_days?: number | null; notes?: string };
type RfqLine = { line: string; name: string; qty: number };
type Rfq = { name: string; scope?: string; config?: string; lines: RfqLine[]; quantities: number[]; due?: string; suppliers: string[]; status: "draft" | "sent" | "closed"; finish?: string; tolerances?: string; notes?: string; created: number };
type QuoteLine = { line: string; prices: PriceBreak[]; lead_days?: number; note?: string };
type Quote = { supplier: string; rfq?: string; reference?: string; lines: QuoteLine[]; currency: string; lead_days?: number; valid_until?: string; received?: string; notes?: string; attachments?: string[]; status: "received" | "accepted" | "declined" };
type Award = { line: string; name: string; supplier: string; quote?: string; prices: PriceBreak[]; currency: string; lead_days?: number };
type Plan = { suppliers: { supplier: string; name: string; lines: { line: string; item: number; name: string; qty: number; missing: boolean }[]; files: string[]; mailto: string; letter: string }[] };

const KINDS = ["3d_print", "cnc_mill", "lathe", "cnc_router", "laser", "sheet_metal", "weld", "cast", "extrusion", "glazing", "distributor"];
const kindName = (k: string) => (k === "distributor" ? "Distributor" : processName(k));

/** Unit price at an order quantity: the largest break at or below it, else the smallest. */
export function priceAt(breaks: PriceBreak[], qty: number): number | null {
  const below = breaks.filter((b) => b.qty <= qty).sort((a, b) => b.qty - a.qty)[0];
  const any = [...breaks].sort((a, b) => a.qty - b.qty)[0];
  return (below ?? any)?.unit_price ?? null;
}

function useKind<T>(entities: ReadonlyMap<string, EntityView>, kind: string) {
  return useMemo(() => {
    const out: [string, T][] = [];
    for (const e of entities.values()) {
      const c = e.components[kind];
      if (c) out.push([e.id, c as T]);
    }
    return out.sort((a, b) => (a[0] < b[0] ? -1 : 1));
  }, [entities, kind]);
}

const today = () => new Date().toISOString().slice(0, 10);

export function SourcingView(props: { entities: ReadonlyMap<string, EntityView>; revision: number; run: Run; notify: (m: string) => void; open: string | null; onOpen: (id: string | null) => void; onBom: () => void }) {
  const suppliers = useKind<Supplier>(props.entities, "sourcing.supplier");
  const rfqs = useKind<Rfq>(props.entities, "sourcing.rfq");
  const quotes = useKind<Quote>(props.entities, "sourcing.quote");
  const awards = useKind<Award>(props.entities, "sourcing.award");
  const [tab, setTab] = useState<"requests" | "suppliers" | "quotes">(props.open || rfqs.length ? "requests" : "suppliers");
  useEffect(() => {
    if (props.open) setTab("requests");
  }, [props.open]);
  const supplierName = (id: string) => suppliers.find(([s]) => s === id)?.[1].name ?? "(deleted supplier)";

  return (
    <div className="sourcing">
      <nav className="src-nav">
        <div className="segmented">
          <button type="button" className={tab === "requests" ? "active" : ""} onClick={() => setTab("requests")}>
            Requests <b>{rfqs.length}</b>
          </button>
          <button type="button" className={tab === "quotes" ? "active" : ""} onClick={() => setTab("quotes")}>
            Quotes <b>{quotes.length}</b>
          </button>
          <button type="button" className={tab === "suppliers" ? "active" : ""} onClick={() => setTab("suppliers")}>
            Suppliers <b>{suppliers.length}</b>
          </button>
        </div>
        <span className="bom-note">Packages and email drafts are made here; you send them.</span>
      </nav>
      {tab === "suppliers" && <Suppliers suppliers={suppliers} run={props.run} />}
      {tab === "requests" && (
        <Requests rfqs={rfqs} suppliers={suppliers} quotes={quotes} awards={awards} revision={props.revision} run={props.run} notify={props.notify} open={props.open} onOpen={props.onOpen} onBom={props.onBom} onSuppliers={() => setTab("suppliers")} supplierName={supplierName} />
      )}
      {tab === "quotes" && <Quotes quotes={quotes} rfqs={rfqs} awards={awards} run={props.run} supplierName={supplierName} onOpen={(id) => (props.onOpen(id), setTab("requests"))} />}
    </div>
  );
}

// ---- Suppliers ----------------------------------------------------------------

const EMPTY: Supplier = { name: "", kind: [], website: "", email: "", phone: "", country: "", currency: "USD", lead_days: null, notes: "" };

function Suppliers({ suppliers, run }: { suppliers: [string, Supplier][]; run: Run }) {
  const [editing, setEditing] = useState<string | "new" | null>(suppliers.length ? null : "new");
  return (
    <div className="suppliers">
      <div className="supplier-grid">
        {suppliers.map(([id, s]) =>
          editing === id ? (
            <SupplierForm key={id} initial={s} onCancel={() => setEditing(null)} onSave={async (v) => (await run("sourcing.supplier.update", { target: id, ...v, phone: v.phone ?? "", lead_days: v.lead_days ?? 0 })) && setEditing(null)} />
          ) : (
            <article key={id} className="supplier-card">
              <header>
                <span className="avatar" aria-hidden="true">
                  {s.name.slice(0, 1).toUpperCase()}
                </span>
                <div>
                  <strong>{s.name}</strong>
                  <small>{[s.country, s.currency, s.lead_days ? `${s.lead_days} days` : ""].filter(Boolean).join(" · ")}</small>
                </div>
                <IconButton icon="pencil" label={`Edit ${s.name}`} onClick={() => setEditing(id)} />
                <IconButton icon="trash" label={`Delete ${s.name}`} onClick={() => run("core.entity.delete", { target: id })} />
              </header>
              <div className="kinds">
                {(s.kind.length ? s.kind : ["any"]).map((k) => (
                  <span key={k} className={`kind-chip ${k}`}>
                    {k === "any" ? "Quotes anything" : kindName(k)}
                  </span>
                ))}
              </div>
              <div className="contact">
                {s.email && (
                  <a href={`mailto:${s.email}`}>
                    <Icon name="mail" /> {s.email}
                  </a>
                )}
                {s.website && (
                  <a href={s.website.startsWith("http") ? s.website : `https://${s.website}`} target="_blank" rel="noreferrer">
                    <Icon name="link" /> {s.website.replace(/^https?:\/\//, "")}
                  </a>
                )}
                {s.phone && <span>{s.phone}</span>}
              </div>
              {s.notes && <p className="muted">{s.notes}</p>}
            </article>
          ),
        )}
        {editing === "new" ? (
          <SupplierForm initial={EMPTY} onCancel={suppliers.length ? () => setEditing(null) : undefined} onSave={async (v) => (await run("sourcing.supplier.create", { ...v, phone: v.phone || null, lead_days: v.lead_days || null })) && setEditing(null)} />
        ) : (
          <button type="button" className="supplier-add" onClick={() => setEditing("new")}>
            <Icon name="plus" />
            <strong>Add a supplier</strong>
            <small>A shop that makes parts, or a distributor of stock items</small>
          </button>
        )}
      </div>
    </div>
  );
}

function SupplierForm({ initial, onSave, onCancel }: { initial: Supplier; onSave: (s: Supplier) => void; onCancel?: () => void }) {
  const [s, setS] = useState<Supplier>({ ...EMPTY, ...initial });
  const set = (patch: Partial<Supplier>) => setS((x) => ({ ...x, ...patch }));
  const toggle = (k: string) => set({ kind: s.kind.includes(k) ? s.kind.filter((x) => x !== k) : [...s.kind, k] });
  return (
    <form
      className="supplier-card editing"
      onSubmit={(e) => {
        e.preventDefault();
        if (s.name.trim()) onSave(s);
      }}
    >
      <div className="form-grid">
        <label className="wide">
          Name
          <input autoFocus value={s.name} onChange={(e) => set({ name: e.target.value })} placeholder="Acme Machining" />
        </label>
        <label>
          Email
          <input type="email" value={s.email ?? ""} onChange={(e) => set({ email: e.target.value })} placeholder="quotes@acme.com" />
        </label>
        <label>
          Website
          <input value={s.website ?? ""} onChange={(e) => set({ website: e.target.value })} placeholder="acme.com" />
        </label>
        <label>
          Phone
          <input value={s.phone ?? ""} onChange={(e) => set({ phone: e.target.value })} />
        </label>
        <label>
          Country
          <input value={s.country ?? ""} onChange={(e) => set({ country: e.target.value })} placeholder="US" />
        </label>
        <label>
          Currency
          <input value={s.currency} maxLength={3} onChange={(e) => set({ currency: e.target.value.toUpperCase() })} />
        </label>
        <label>
          Lead time, days
          <input inputMode="numeric" value={s.lead_days ?? ""} onChange={(e) => set({ lead_days: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })} />
        </label>
      </div>
      <div className="kind-pick" role="group" aria-label="What they do">
        <span>What they do</span>
        {KINDS.map((k) => (
          <button type="button" key={k} className={`kind-chip ${k} ${s.kind.includes(k) ? "on" : ""}`} aria-pressed={s.kind.includes(k)} onClick={() => toggle(k)}>
            {kindName(k)}
          </button>
        ))}
      </div>
      <label className="wide">
        Notes
        <textarea rows={2} value={s.notes ?? ""} onChange={(e) => set({ notes: e.target.value })} placeholder="Payment terms, minimum order, who to ask for" />
      </label>
      <div className="row-buttons">
        <button type="submit" className="primary" disabled={!s.name.trim()}>
          <Icon name="check" /> Save supplier
        </button>
        {onCancel && (
          <button type="button" className="secondary" onClick={onCancel}>
            Cancel
          </button>
        )}
      </div>
    </form>
  );
}

// ---- Requests -----------------------------------------------------------------

const STATUS_WORDS: Record<Rfq["status"], string> = { draft: "Draft", sent: "Sent", closed: "Closed" };

function Requests(props: {
  rfqs: [string, Rfq][];
  suppliers: [string, Supplier][];
  quotes: [string, Quote][];
  awards: [string, Award][];
  revision: number;
  run: Run;
  notify: (m: string) => void;
  open: string | null;
  onOpen: (id: string | null) => void;
  onBom: () => void;
  onSuppliers: () => void;
  supplierName: (id: string) => string;
}) {
  const sorted = [...props.rfqs].sort((a, b) => b[1].created - a[1].created);
  const current = sorted.find(([id]) => id === props.open) ?? sorted[0];
  if (!current) {
    return (
      <div className="src-empty">
        <Icon name="mail" />
        <strong>No requests for quote yet</strong>
        <p>Choose lines in the bill of materials and press Request quotes. Each supplier gets only the lines they make, with the files their shop needs.</p>
        <button type="button" className="primary" onClick={props.onBom}>
          <Icon name="list" /> Open the bill of materials
        </button>
      </div>
    );
  }
  return (
    <div className="requests">
      <aside className="rfq-list">
        {sorted.map(([id, r]) => {
          const n = props.quotes.filter(([, q]) => q.rfq === id).length;
          return (
            <button type="button" key={id} className={id === current[0] ? "on" : ""} onClick={() => props.onOpen(id)}>
              <span className={`status ${r.status}`}>{STATUS_WORDS[r.status]}</span>
              <strong>{r.name}</strong>
              <small>
                {r.lines.length} line{r.lines.length === 1 ? "" : "s"} · {r.suppliers.length} supplier{r.suppliers.length === 1 ? "" : "s"} · {n} quote{n === 1 ? "" : "s"}
                {r.due ? ` · due ${r.due}` : ""}
              </small>
            </button>
          );
        })}
      </aside>
      <RfqDetail key={current[0]} id={current[0]} rfq={current[1]} {...props} />
    </div>
  );
}

function Field({ label, value, onCommit, type = "text", placeholder, multiline }: { label: string; value: string; onCommit: (v: string) => void; type?: string; placeholder?: string; multiline?: boolean }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = () => v !== value && onCommit(v);
  return (
    <label className={multiline ? "wide" : ""}>
      {label}
      {multiline ? (
        <textarea rows={2} value={v} placeholder={placeholder} onChange={(e) => setV(e.target.value)} onBlur={commit} />
      ) : (
        <input type={type} value={v} placeholder={placeholder} onChange={(e) => setV(e.target.value)} onBlur={commit} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      )}
    </label>
  );
}

function RfqDetail(props: {
  id: string;
  rfq: Rfq;
  suppliers: [string, Supplier][];
  quotes: [string, Quote][];
  awards: [string, Award][];
  revision: number;
  run: Run;
  notify: (m: string) => void;
  onOpen: (id: string | null) => void;
  onSuppliers: () => void;
  supplierName: (id: string) => string;
}) {
  const { id, rfq, run } = props;
  const [plan, setPlan] = useState<Plan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [letter, setLetter] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const update = (patch: Record<string, unknown>) => run("sourcing.rfq.update", { target: id, ...patch });

  useEffect(() => {
    if (rfq.suppliers.length === 0) {
      setPlan(null);
      setPlanError(null);
      return;
    }
    let live = true;
    fetch(projectUrl(`/rfq/${id}`))
      .then(async (r) => (r.ok ? (r.json() as Promise<Plan>) : Promise.reject(new Error(await r.text()))))
      .then((p) => live && (setPlan(p), setPlanError(null)))
      .catch((e: Error) => live && setPlanError(e.message));
    return () => {
      live = false;
    };
  }, [id, props.revision, rfq.suppliers.length]);

  const rfqQuotes = props.quotes.filter(([, q]) => q.rfq === id);
  return (
    <div className="rfq-detail">
      <header className="rfq-head">
        <input className="rfq-title" defaultValue={rfq.name} key={rfq.name} aria-label="Request name" onBlur={(e) => e.target.value.trim() && e.target.value !== rfq.name && update({ name: e.target.value })} />
        <div className="segmented small">
          {(["draft", "sent", "closed"] as const).map((s) => (
            <button type="button" key={s} className={rfq.status === s ? "active" : ""} onClick={() => update({ status: s })}>
              {STATUS_WORDS[s]}
            </button>
          ))}
        </div>
        <IconButton
          icon="trash"
          label="Delete this request"
          onClick={async () => {
            if (await run("core.entity.delete", { target: id })) props.onOpen(null);
          }}
        />
      </header>

      <div className="form-grid rfq-fields">
        <Field label="Quotes wanted by" type="date" value={rfq.due ?? ""} onCommit={(v) => update({ due: v })} />
        <Field
          label="Production quantities"
          value={rfq.quantities.join(", ")}
          placeholder="1, 10, 100"
          onCommit={(v) => {
            const q = v.split(/[ ,;]+/).map(Number).filter((n) => Number.isInteger(n) && n > 0);
            if (q.length) update({ quantities: q });
          }}
        />
        <Field label="Finish" value={rfq.finish ?? ""} placeholder="As machined, deburred" onCommit={(v) => update({ finish: v })} />
        <Field label="Tolerances" value={rfq.tolerances ?? ""} placeholder="ISO 2768-m unless the drawing says otherwise" onCommit={(v) => update({ tolerances: v })} />
        <Field label="Notes for suppliers" value={rfq.notes ?? ""} multiline onCommit={(v) => update({ notes: v })} />
      </div>

      <h3 className="bench-section">Suppliers</h3>
      <div className="supplier-pick">
        {props.suppliers.map(([sid, s]) => {
          const on = rfq.suppliers.includes(sid);
          return (
            <button type="button" key={sid} className={`pick ${on ? "on" : ""}`} aria-pressed={on} onClick={() => update({ suppliers: on ? rfq.suppliers.filter((x) => x !== sid) : [...rfq.suppliers, sid] })}>
              {on && <Icon name="check" />}
              <strong>{s.name}</strong>
              <small>{s.kind.length ? s.kind.map(kindName).join(", ") : "Anything"}</small>
            </button>
          );
        })}
        <button type="button" className="pick add" onClick={props.onSuppliers}>
          <Icon name="plus" /> Add a supplier
        </button>
      </div>

      {rfq.suppliers.length > 0 && (
        <>
          <h3 className="bench-section">
            Packages
            <a className="secondary" href={projectUrl(`/rfq/${id}.zip`)} download>
              <Icon name="download" /> Download all
            </a>
          </h3>
          {planError && <p className="fix">{planError}</p>}
          <div className="packages">
            {(plan?.suppliers ?? []).map((p) => (
              <div key={p.supplier} className={`package ${p.lines.length ? "" : "empty"}`}>
                <div className="package-main">
                  <strong>{p.name}</strong>
                  <small>
                    {p.lines.length ? `${p.lines.length} line${p.lines.length === 1 ? "" : "s"}: ${p.lines.map((l) => l.name).join(", ")}` : "None of these lines are theirs to quote"}
                  </small>
                  <div className="package-files">
                    {p.files.map((f) => (
                      <span key={f} className="file-chip">
                        {f}
                      </span>
                    ))}
                    <span className="file-chip quiet">BOM.csv</span>
                    <span className="file-chip quiet">RFQ.md</span>
                  </div>
                </div>
                <div className="package-actions">
                  <button type="button" className="ib-text" onClick={() => setLetter(letter === p.letter ? null : p.letter)}>
                    Letter
                  </button>
                  <a className="secondary" href={p.mailto} title="Opens a draft in your mail app; attach the package and send it yourself">
                    <Icon name="mail" /> Email draft
                  </a>
                  <a className="primary" href={projectUrl(`/rfq/${id}.zip?supplier=${p.supplier}`)} download>
                    <Icon name="download" /> Package
                  </a>
                </div>
              </div>
            ))}
          </div>
          {letter && <pre className="letter">{letter}</pre>}
        </>
      )}

      <h3 className="bench-section">
        Quotes
        {rfq.suppliers.length > 0 && (
          <button type="button" className="secondary" onClick={() => setRecording((r) => !r)}>
            <Icon name="plus" /> Record a quote
          </button>
        )}
      </h3>
      {recording && <RecordQuote rfqId={id} rfq={rfq} suppliers={props.suppliers} run={run} onDone={() => setRecording(false)} />}
      <Compare rfq={rfq} quotes={rfqQuotes} awards={props.awards} run={run} supplierName={props.supplierName} />
    </div>
  );
}

function RecordQuote({ rfqId, rfq, suppliers, run, onDone }: { rfqId: string; rfq: Rfq; suppliers: [string, Supplier][]; run: Run; onDone: () => void }) {
  const asked = suppliers.filter(([id]) => rfq.suppliers.includes(id));
  const [supplier, setSupplier] = useState(asked[0]?.[0] ?? "");
  const [reference, setReference] = useState("");
  const [lead, setLead] = useState("");
  const [valid, setValid] = useState("");
  const [prices, setPrices] = useState<Record<string, string>>({});
  const s = suppliers.find(([id]) => id === supplier)?.[1];
  const key = (line: string, q: number) => `${line}@${q}`;
  const save = async () => {
    const lines = rfq.lines
      .map((l) => ({
        line: l.line,
        prices: rfq.quantities
          .map((q) => ({ qty: l.qty * q, unit_price: Number(prices[key(l.line, q)]) }))
          .filter((p, i) => prices[key(l.line, rfq.quantities[i])]?.trim() && Number.isFinite(p.unit_price) && p.unit_price >= 0),
      }))
      .filter((l) => l.prices.length);
    if (!supplier || lines.length === 0) return;
    const r = await run("sourcing.quote.create", { supplier, rfq: rfqId, reference, lines, lead_days: lead ? Number(lead) : null, valid_until: valid || null, received: today(), currency: s?.currency ?? "" });
    if (r) onDone();
  };
  return (
    <div className="record-quote">
      <div className="form-grid">
        <label>
          Supplier
          <select value={supplier} onChange={(e) => setSupplier(e.target.value)}>
            {asked.map(([id, x]) => (
              <option key={id} value={id}>
                {x.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Their reference
          <input value={reference} onChange={(e) => setReference(e.target.value)} placeholder="Q-2041" />
        </label>
        <label>
          Lead time, days
          <input inputMode="numeric" value={lead} onChange={(e) => setLead(e.target.value.replace(/\D/g, ""))} />
        </label>
        <label>
          Valid until
          <input type="date" value={valid} onChange={(e) => setValid(e.target.value)} />
        </label>
      </div>
      <table className="price-grid">
        <thead>
          <tr>
            <th>Line</th>
            {rfq.quantities.map((q) => (
              <th key={q} className="num">
                {q.toLocaleString()} units
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rfq.lines.map((l) => (
            <tr key={l.line}>
              <td>
                <strong>{l.name}</strong>
                <small>{l.qty} per unit</small>
              </td>
              {rfq.quantities.map((q) => (
                <td key={q}>
                  <span className="price-in">
                    <small>{(l.qty * q).toLocaleString()} @</small>
                    <input inputMode="decimal" placeholder="—" value={prices[key(l.line, q)] ?? ""} onChange={(e) => setPrices((p) => ({ ...p, [key(l.line, q)]: e.target.value.replace(/[^0-9.]/g, "") }))} aria-label={`${l.name}, unit price for ${l.qty * q}`} />
                    <small>{s?.currency ?? ""}</small>
                  </span>
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="row-buttons">
        <button type="button" className="primary" onClick={save} disabled={!supplier}>
          <Icon name="check" /> Save quote
        </button>
        <button type="button" className="secondary" onClick={onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

/** Quotes for one RFQ side by side, per line, at one production quantity: cheapest and fastest marked, award in a click. */
function Compare({ rfq, quotes, awards, run, supplierName }: { rfq: Rfq; quotes: [string, Quote][]; awards: [string, Award][]; run: Run; supplierName: (id: string) => string }) {
  const [q, setQ] = useState(rfq.quantities[Math.min(1, rfq.quantities.length - 1)] ?? 1);
  if (quotes.length === 0) {
    return <p className="muted compare-empty">No quotes yet. When a supplier answers, record their prices here and compare them line by line.</p>;
  }
  const live = quotes.filter(([, x]) => x.status !== "declined");
  const cell = (quote: Quote, l: RfqLine) => {
    const ql = quote.lines.find((x) => x.line === l.line);
    if (!ql || ql.prices.length === 0) return null;
    const unit = priceAt(ql.prices, l.qty * q);
    return unit == null ? null : { unit, ext: unit * l.qty * q, lead: ql.lead_days ?? quote.lead_days ?? null };
  };
  const awardOf = (line: string) => awards.find(([, a]) => a.line === line)?.[1];
  const best = (l: RfqLine) => {
    const cells = live.map(([id, x]) => ({ id, c: cell(x, l) })).filter((c) => c.c);
    const cheapest = cells.length ? cells.reduce((a, b) => (b.c!.unit < a.c!.unit ? b : a)).id : null;
    const timed = cells.filter((c) => c.c!.lead != null);
    const fastest = timed.length > 1 ? timed.reduce((a, b) => (b.c!.lead! < a.c!.lead! ? b : a)).id : null;
    return { cheapest, fastest };
  };
  const awardCheapest = async () => {
    for (const l of rfq.lines) {
      const { cheapest } = best(l);
      if (cheapest && awardOf(l.line)?.quote !== cheapest) await run("sourcing.award", { line: l.line, quote: cheapest, name: l.name });
    }
  };
  return (
    <div className="compare">
      <div className="compare-bar">
        <div className="segmented small" role="group" aria-label="Production quantity">
          {rfq.quantities.map((n) => (
            <button type="button" key={n} className={n === q ? "active" : ""} onClick={() => setQ(n)}>
              ×{n.toLocaleString()}
            </button>
          ))}
        </div>
        <span className="legend">
          <i className="cheap" /> cheapest <i className="fast" /> fastest
        </span>
        <button type="button" className="secondary" onClick={awardCheapest}>
          <Icon name="check" /> Award cheapest per line
        </button>
      </div>
      <div className="bom-table-wrap">
        <table className="bom-table compare-table">
          <thead>
            <tr>
              <th>Line</th>
              <th className="num">Qty</th>
              {quotes.map(([id, x]) => (
                <th key={id} className={`quote-col ${x.status}`}>
                  <strong>{supplierName(x.supplier)}</strong>
                  <small>
                    {x.reference || "—"}
                    {x.lead_days ? ` · ${x.lead_days} d` : ""}
                    {x.valid_until ? ` · until ${x.valid_until}` : ""}
                  </small>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rfq.lines.map((l) => {
              const { cheapest, fastest } = best(l);
              const award = awardOf(l.line);
              return (
                <tr key={l.line}>
                  <td className="c-name">
                    <strong>{l.name}</strong>
                    {award && <small className="awarded-to">Awarded · {supplierName(award.supplier)}</small>}
                  </td>
                  <td className="num">{(l.qty * q).toLocaleString()}</td>
                  {quotes.map(([id, x]) => {
                    const c = cell(x, l);
                    const won = award?.quote === id;
                    if (!c) return <td key={id} className="q-cell none">—</td>;
                    return (
                      <td key={id} className={`q-cell ${cheapest === id ? "cheap" : ""} ${fastest === id ? "fast" : ""} ${won ? "won" : ""} ${x.status === "declined" ? "declined" : ""}`}>
                        <div>
                          <b>{money(c.unit, x.currency)}</b>
                          <small>{money(c.ext, x.currency, 0)}</small>
                        </div>
                        <div className="q-tags">
                          {c.lead != null && <span className="lead">{c.lead} d</span>}
                          {won ? (
                            <button type="button" className="award on" onClick={() => run("sourcing.award.clear", { line: l.line })} title="Withdraw the award">
                              <Icon name="check" /> Awarded
                            </button>
                          ) : (
                            x.status !== "declined" && (
                              <button type="button" className="award" onClick={() => run("sourcing.award", { line: l.line, quote: id, name: l.name })}>
                                Award
                              </button>
                            )
                          )}
                        </div>
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
          <tfoot>
            <tr>
              <td colSpan={2}>Total at ×{q.toLocaleString()}</td>
              {quotes.map(([id, x]) => {
                const cells = rfq.lines.map((l) => cell(x, l));
                const total = cells.reduce((s, c) => s + (c?.ext ?? 0), 0);
                const covered = cells.filter(Boolean).length;
                return (
                  <td key={id} className="num">
                    <b>{money(total, x.currency, 0)}</b>
                    <small>
                      {covered}/{rfq.lines.length} lines
                    </small>
                  </td>
                );
              })}
            </tr>
          </tfoot>
        </table>
      </div>
    </div>
  );
}

// ---- All quotes ---------------------------------------------------------------

function Quotes({ quotes, rfqs, awards, run, supplierName, onOpen }: { quotes: [string, Quote][]; rfqs: [string, Rfq][]; awards: [string, Award][]; run: Run; supplierName: (id: string) => string; onOpen: (rfq: string) => void }) {
  if (quotes.length === 0) {
    return (
      <div className="src-empty">
        <Icon name="tag" />
        <strong>No quotes yet</strong>
        <p>Record a supplier's prices on the request they answer; they show here, and the BOM uses the ones you award.</p>
      </div>
    );
  }
  return (
    <div className="bom-table-wrap quotes-list">
      <table className="bom-table">
        <thead>
          <tr>
            <th>Supplier</th>
            <th>Request</th>
            <th>Reference</th>
            <th className="num">Lines</th>
            <th className="num">Lead</th>
            <th>Valid until</th>
            <th>Status</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {[...quotes].reverse().map(([id, q]) => {
            const rfq = rfqs.find(([r]) => r === q.rfq);
            const won = awards.filter(([, a]) => a.quote === id).length;
            const expired = q.valid_until && q.valid_until < today();
            return (
              <tr key={id}>
                <td className="c-name">
                  <strong>{supplierName(q.supplier)}</strong>
                  <small>{q.received ? `received ${q.received}` : ""}</small>
                </td>
                <td>
                  {rfq ? (
                    <button type="button" className="item-link" onClick={() => onOpen(rfq[0])}>
                      {rfq[1].name}
                    </button>
                  ) : (
                    <span className="none">—</span>
                  )}
                </td>
                <td>{q.reference || <span className="none">—</span>}</td>
                <td className="num">
                  {q.lines.length}
                  {won > 0 && <small>{won} awarded</small>}
                </td>
                <td className="num">{q.lead_days ? `${q.lead_days} d` : "—"}</td>
                <td className={expired ? "expired" : ""}>{q.valid_until ?? "—"}</td>
                <td>
                  <span className={`status ${q.status}`}>{q.status === "received" ? "Received" : q.status === "accepted" ? "Accepted" : "Declined"}</span>
                </td>
                <td className="row-actions">
                  {q.status !== "accepted" && (
                    <button type="button" className="ib-text" onClick={() => run("sourcing.quote.award", { quote: id, names: Object.fromEntries((rfq?.[1].lines ?? []).map((l) => [l.line, l.name])) })}>
                      Accept all
                    </button>
                  )}
                  {q.status !== "declined" && (
                    <button type="button" className="ib-text" onClick={() => run("sourcing.quote.update", { target: id, status: "declined" })}>
                      Decline
                    </button>
                  )}
                  <IconButton icon="trash" label="Delete quote" onClick={() => run("core.entity.delete", { target: id })} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

