// The Building tab of the workbench: the building's levels and rooms, its
// schedules (quantity take-off by level, class or type), IFC out, and its
// plans, sections and elevations. The engine gathers the model from every
// placed shape and works all of it out; this draws it.

import { useEffect, useMemo, useState } from "react";
import { type EntityView, projectUrl } from "./engine";
import { Icon } from "./icons";
import { instance, program } from "./model";

type Pt = [number, number];
type Pen = "visible" | "hidden" | "thin" | "center" | "frame";
type Sheet = { width: number; height: number; scale_text: string; strokes: { pen: Pen; points: Pt[] }[]; fills: Pt[][]; texts: { at: Pt; size: number; text: string; anchor: "start" | "middle" | "end"; angle: number; bold: boolean }[]; warnings: string[] };
type Structure = {
  project: string;
  site: { name: string; latitude?: number; longitude?: number };
  building: { name: string; address?: string };
  levels: { name: string; elevation: number; height?: number; elements: Record<string, number>; spaces: { name: string; long_name?: string; area_m2: number; height: number }[] }[];
  classes: Record<string, number>;
  elements: number;
  openings: number;
  bounds: (number | null)[];
  zones: { name: string; spaces: string[] }[];
  grids: { name: string; level?: string; axes: string[] }[];
  untagged: string[];
  notes: string[];
};
type Row = { name: string; class: string; type?: string; level?: string; tag?: string; material?: string; host?: string; quantities: Record<string, number> };
type Total = { key: string[]; count: number; sums: Record<string, number> };
type Schedule = { rows: Row[]; columns: string[]; units: Record<string, string>; group_by: string[]; groups: Total[]; total: Total };

const WIDTH: Record<Pen, number> = { visible: 0.5, hidden: 0.25, thin: 0.18, center: 0.18, frame: 0.7 };
const DASH: Partial<Record<Pen, string>> = { hidden: "2 1", center: "8 1.5 0.5 1.5" };

const FILTERS: { label: string; classes: string }[] = [
  { label: "Everything", classes: "" },
  { label: "Walls", classes: "IfcWall" },
  { label: "Doors", classes: "IfcDoor" },
  { label: "Windows", classes: "IfcWindow" },
  { label: "Slabs & roofs", classes: "IfcSlab,IfcRoof" },
  { label: "Stairs", classes: "IfcStair,IfcRamp,IfcRailing" },
  { label: "Structure", classes: "IfcColumn,IfcBeam,IfcMember,IfcFooting" },
  { label: "Rooms", classes: "IfcSpace" },
  { label: "Zones", classes: "IfcZone" },
  { label: "Rebar", classes: "IfcReinforcingBar" },
];
const GROUPS: { label: string; value: string }[] = [
  { label: "Each item", value: "" },
  { label: "By level", value: "level" },
  { label: "By class", value: "class" },
  { label: "By level and class", value: "level,class" },
  { label: "By type", value: "type" },
  { label: "By material", value: "material" },
];

const short = (c: string) => c.replace(/^Ifc/, "").replace(/([a-z])([A-Z])/g, "$1 $2");
const fmt = (v: number | undefined, unit: string) => (v === undefined ? "" : unit === "m²" || unit === "m³" ? v.toFixed(2) : unit === "kg" ? v.toFixed(1) : String(Math.round(v)));

function useJson<T>(url: string | null, revision: number): { data: T | null; error: string | null; loading: boolean } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!url) {
      setData(null);
      return;
    }
    let live = true;
    setLoading(true);
    fetch(projectUrl(url))
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        return (await r.json()) as T;
      })
      .then((d) => {
        if (!live) return;
        setData(d);
        setError(null);
      })
      .catch((e: Error) => live && setError(e.message))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [url, revision]);
  return { data, error, loading };
}

function SheetView({ d }: { d: Sheet }) {
  const flip = (p: Pt) => `${p[0].toFixed(2)},${(d.height - p[1]).toFixed(2)}`;
  return (
    <svg className="bim-sheet" viewBox={`0 0 ${d.width} ${d.height}`} role="img" aria-label="Drawing sheet">
      <rect width={d.width} height={d.height} fill="#fff" />
      {/* Every fill one path: the triangles of a cut wall show no seams. */}
      {d.fills.length > 0 && <path d={d.fills.map((f) => `M${f.map(flip).join(" L")} Z`).join(" ")} fill="#1d1a24" />}
      {d.strokes.map((s, i) => (
        <polyline key={i} points={s.points.map(flip).join(" ")} fill="none" stroke="#1d1a24" strokeWidth={WIDTH[s.pen]} strokeDasharray={DASH[s.pen]} strokeLinecap="round" strokeLinejoin="round" />
      ))}
      {d.texts.map((t, i) => (
        <text
          key={`t${i}`}
          x={t.at[0]}
          y={d.height - t.at[1]}
          fontSize={t.size / 0.718}
          fontWeight={t.bold ? 700 : 400}
          textAnchor={t.anchor}
          transform={t.angle ? `rotate(${-t.angle} ${t.at[0]} ${d.height - t.at[1]})` : undefined}
          fill="#1d1a24"
          fontFamily="Helvetica, Arial, sans-serif"
        >
          {t.text}
        </text>
      ))}
    </svg>
  );
}

export function BuildingView(props: { entities: ReadonlyMap<string, EntityView>; revision: number; selected: string | null }) {
  const picked = useMemo(() => {
    const e = props.selected ? props.entities.get(props.selected) : undefined;
    if (!e) return null;
    const name = instance(e)?.name ?? program(e)?.name;
    return name ? { id: e.id, name } : null;
  }, [props.entities, props.selected]);
  const [useSelection, setUseSelection] = useState(false);
  const scope = useSelection && picked ? picked.id : "project";
  const { data: b, error } = useJson<Structure>(`/bim/${scope}`, props.revision);
  const [filter, setFilter] = useState(0);
  const [group, setGroup] = useState("level,class");
  const classes = FILTERS[filter].classes;
  const q = new URLSearchParams({ ...(classes ? { class: classes } : {}), ...(group ? { group } : {}) });
  const { data: sched, loading: schedLoading } = useJson<{ schedule: Schedule }>(b ? `/schedule/${scope}?${q}` : null, props.revision);
  const s = sched?.schedule;
  // Drawings: each level's plan, the four elevations, two sections through the middle.
  const views = useMemo(() => {
    if (!b) return [];
    const out: { label: string; query: string }[] = b.levels.map((l) => ({ label: `${l.name} plan`, query: `view=plan&level=${encodeURIComponent(l.name)}` }));
    for (const side of ["south", "east", "north", "west"]) out.push({ label: `${side[0].toUpperCase()}${side.slice(1)} elevation`, query: `view=elevation&side=${side}` });
    const [x0, y0, , x1, y1] = b.bounds as number[];
    if (Number.isFinite(x0)) {
      out.push({ label: "Section A–A (east–west)", query: `view=section&axis=y&at=${Math.round((y0 + y1) / 2)}&looking=north` });
      out.push({ label: "Section B–B (north–south)", query: `view=section&axis=x&at=${Math.round((x0 + x1) / 2)}&looking=east` });
    }
    return out;
  }, [b]);
  const [viewIndex, setViewIndex] = useState(0);
  const view = views[Math.min(viewIndex, Math.max(0, views.length - 1))];
  const { data: sheet, error: sheetError, loading: sheetLoading } = useJson<Sheet>(view ? `/plan/${scope}?${view.query}` : null, props.revision);
  // Totals show what adds up and matters; single rows show the first columns they have.
  const MAIN = ["Length", "NetSideArea", "NetArea", "NetFloorArea", "Area", "NetVolume", "Volume", "TotalLength", "Weight", "Openings"];
  const cols = s
    ? s.group_by.length
      ? MAIN.filter((c) => s.groups.some((g) => g.sums[c] !== undefined))
      : s.columns.filter((c) => s.rows.some((r) => r.quantities[c] !== undefined)).slice(0, 9)
    : [];
  const head = (c: string) => `${c.replace(/([a-z])([A-Z])/g, "$1 $2")}${s?.units[c] ? ` (${s.units[c]})` : ""}`;

  return (
    <div className="bom building">
      <div className="bom-bar">
        <div className="bom-scope">
          <button type="button" className={`scope-chip ${scope === "project" ? "on" : ""}`} onClick={() => setUseSelection(false)}>
            <Icon name="layers" /> Whole project
          </button>
          {picked && (
            <button type="button" className={`scope-chip ${scope !== "project" ? "on" : ""}`} onClick={() => setUseSelection(true)}>
              <Icon name="cube" /> {picked.name}
            </button>
          )}
        </div>
        <div className="bom-actions">
          <a className="download" href={projectUrl(`/ifc/${scope}.ifc`)} download title="IFC4: site, building, storeys, elements with geometry, openings, spaces, zones, grids, materials, property sets and quantities">
            <Icon name="download" /> IFC
          </a>
          <a className="download" href={projectUrl(`/schedule/${scope}?${q}&format=csv`)} download title="This schedule as CSV">
            <Icon name="download" /> Schedule CSV
          </a>
        </div>
      </div>
      {error && <p className="bom-note bim-error">{error}</p>}
      {b && (
        <div className="bim-body">
          <section className="bim-levels" aria-label="Levels">
            <h3>
              {b.building.name || b.project} <span>{b.site.name}</span>
            </h3>
            {[...b.levels].reverse().map((l) => (
              <div key={l.name} className="bim-level">
                <div className="bim-level-head">
                  <strong>{l.name}</strong>
                  <span>
                    {l.elevation >= 0 ? "+" : ""}
                    {(l.elevation / 1000).toFixed(3)} m
                  </span>
                </div>
                <div className="bim-level-els">
                  {Object.entries(l.elements)
                    .map(([c, n]) => `${n} ${short(c).toLowerCase()}${n > 1 && !/s$/.test(c) ? "s" : ""}`)
                    .join(" · ") || "—"}
                </div>
                {l.spaces.map((sp) => (
                  <div key={sp.name} className="bim-space">
                    <span>{sp.long_name ?? sp.name}</span>
                    <span>{sp.area_m2.toFixed(1)} m²</span>
                  </div>
                ))}
              </div>
            ))}
            {b.zones.map((z) => (
              <div key={z.name} className="bim-zone">
                Zone <strong>{z.name}</strong>: {z.spaces.join(", ")}
              </div>
            ))}
            {b.untagged.length > 0 && <p className="bom-note">No IFC class (exported as proxies): {b.untagged.join(", ")}</p>}
            {b.notes.map((n) => (
              <p key={n} className="bom-note">
                {n}
              </p>
            ))}
          </section>
          <section className="bim-schedule" aria-label="Schedule">
            <div className="bim-controls">
              <div className="bim-filters">
                {FILTERS.map((f, i) => (
                  <button key={f.label} type="button" className={`scope-chip ${filter === i ? "on" : ""}`} onClick={() => setFilter(i)}>
                    {f.label}
                  </button>
                ))}
              </div>
              <select value={group} onChange={(e) => setGroup(e.target.value)} aria-label="Group by">
                {GROUPS.map((g) => (
                  <option key={g.value} value={g.value}>
                    {g.label}
                  </option>
                ))}
              </select>
            </div>
            <div className={`bim-table-wrap ${schedLoading ? "loading" : ""}`}>
              {s && (
                <table className="bim-table">
                  <thead>
                    <tr>
                      {s.group_by.length ? s.group_by.map((g) => <th key={g}>{g[0].toUpperCase() + g.slice(1)}</th>) : [<th key="l">Level</th>, <th key="c">Class</th>, <th key="n">Name</th>, <th key="t">Type</th>]}
                      <th className="num">Count</th>
                      {cols.map((c) => (
                        <th key={c} className="num">
                          {head(c)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {s.group_by.length
                      ? s.groups.map((g) => (
                          <tr key={g.key.join("|")}>
                            {g.key.map((k, i) => (
                              <td key={i}>{s.group_by[i] === "class" ? short(k) : k}</td>
                            ))}
                            <td className="num">{g.count}</td>
                            {cols.map((c) => (
                              <td key={c} className="num">
                                {fmt(g.sums[c], s.units[c])}
                              </td>
                            ))}
                          </tr>
                        ))
                      : s.rows.map((r) => (
                          <tr key={r.name}>
                            <td>{r.level ?? "—"}</td>
                            <td>{short(r.class)}</td>
                            <td>
                              {r.name}
                              {r.tag && r.tag !== r.name ? <span className="bim-tag">{r.tag}</span> : null}
                            </td>
                            <td className="bim-type">{r.type ?? ""}</td>
                            <td className="num">1</td>
                            {cols.map((c) => (
                              <td key={c} className="num">
                                {fmt(r.quantities[c], s.units[c])}
                              </td>
                            ))}
                          </tr>
                        ))}
                  </tbody>
                  <tfoot>
                    <tr>
                      <td colSpan={s.group_by.length || 4}>Total</td>
                      <td className="num">{s.total.count}</td>
                      {cols.map((c) => (
                        <td key={c} className="num">
                          {fmt(s.total.sums[c], s.units[c])}
                        </td>
                      ))}
                    </tr>
                  </tfoot>
                </table>
              )}
            </div>
          </section>
          <section className="bim-drawing" aria-label="Drawings">
            <div className="bim-controls">
              <select value={viewIndex} onChange={(e) => setViewIndex(Number(e.target.value))} aria-label="Drawing">
                {views.map((v, i) => (
                  <option key={v.label} value={i}>
                    {v.label}
                  </option>
                ))}
              </select>
              {view && (
                <>
                  <a className="download" href={projectUrl(`/plan/${scope}.pdf?${view.query}`)} download>
                    <Icon name="download" /> PDF
                  </a>
                  <a className="download" href={projectUrl(`/plan/${scope}.dxf?${view.query}`)} download>
                    <Icon name="download" /> DXF
                  </a>
                </>
              )}
              {sheet && <span className="bom-note">{sheet.scale_text}</span>}
            </div>
            <div className={`bim-sheet-wrap ${sheetLoading ? "loading" : ""}`}>
              {sheetError ? <p className="bom-note bim-error">{sheetError}</p> : sheet ? <SheetView d={sheet} /> : null}
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
