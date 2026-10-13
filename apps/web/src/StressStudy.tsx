// A stress study on a part: pick the faces it is held by and the faces
// loads push on, run it, and read the stress map, the largest stress, how
// far it bends, and the safety factor against its material's yield. Parts
// of an assembly that touch it can be studied with it: they press on it
// where they touch (sliding freely, gripping with friction, or glued), and
// the contact pressure shows as a layer of its own. Besides plain holds and
// face loads: sliding faces, springs, set movements, bearing loads, moments,
// gravity and spinning; letting it yield; buckling, prestressed vibration,
// fatigue, a convergence check, and the result read back at a point, cut
// open, or taken away as a table or a CalculiX deck.

import { Fragment, useEffect, useMemo, useState } from "react";
import { type PartStats, projectUrl } from "./engine";
import { Icon } from "./icons";
import { MaterialProperties, type MaterialProps } from "./MaterialProperties";
import type { ProgramFace, StressView } from "./Viewport";

type V3 = [number, number, number];
/** `curved`: a round face (a cylinder's side, a bore, a fillet), each triangle facing its own way. */
type Face = { point: V3; normal: V3; triangles: number[]; part?: string | null; curved?: boolean };
/** A face as kept with the study: where on the part it sits (fractions of the part's bounds) and which way it faces; `part` when on another part studied with it; `curved` for the whole round surface under the probe. */
type Pick = { at: V3; probe: V3; normal: V3; part?: string; curved?: boolean };
type ContactKind = "frictionless" | "friction" | "bonded";
type ContactSpec = { parts: [string, string]; kind: ContactKind; friction: number; press_fit?: boolean };
type Convection = { h: number; ambient: number };
type FatigueSpec = { ratio: number; cycles: number; surface: string; reliability: number; diameter?: number | null };
type Spec = {
  part: string | null;
  supports: Pick[];
  loads: { face: Pick; magnitude: number; direction: Direction }[];
  holds: { face: Pick; temperature: number }[];
  heats: { face: Pick; watts: number }[];
  convection: Convection | null;
  resolution: number;
  /** Other parts studied with it, and how each pair acts where they touch. */
  parts: string[];
  contacts: ContactSpec[];
  /** The heat study's temperatures load the stress study, stress-free at `reference` °C. */
  heat_stress?: { reference: number } | null;
  /** Re-solve the most stressed spot on finer cubes for its true peak. */
  refine?: boolean;
  /** Faces held square to themselves only (symmetry planes, frictionless supports). */
  sliding?: Pick[];
  /** Faces on springs (N/mm per mm²). */
  elastic_supports?: { face: Pick; stiffness: number; normal_only: boolean }[];
  /** Faces moved by set amounts (mm; null: free that way). */
  displacements?: { face: Pick; by: (number | null)[] }[];
  /** A shaft pushing in a bore: the force spread over the half it pushes into. */
  bearing_loads?: { face: Pick; force: V3 }[];
  /** A force and moment at a point, carried to a face (here: a moment about the face's middle). */
  remote_loads?: { face: Pick; point: V3; force: V3; moment: V3 }[];
  gravity?: V3 | null;
  spin?: { at: V3; axis: V3; rpm: number } | null;
  plastic?: boolean;
  prestress?: boolean;
  fatigue?: FatigueSpec | null;
};
/** The saved faces found on the part as it is now (null: no longer there). */
/** A load's `pressure` (MPa) pushes every bit of its face straight in, besides its `force` (N). */
type Resolved = { supports: (Face | null)[]; loads: { face: Face | null; force: V3 | null; pressure?: number }[]; holds: { face: Face | null }[]; heats: { face: Face | null }[] };
type Temps = { cell: number; origin: V3; cells: [V3, number][]; max: number; min: number; heat_in: number; to_holds: number; to_air: number; notes: string[] };
const AIR: { label: string; h: number }[] = [
  { label: "No air", h: 0 },
  { label: "Still air", h: 10 },
  { label: "Fan", h: 50 },
];
type Direction = "into" | "pressure" | "-z" | "+z" | "-x" | "+x" | "-y" | "+y";
type PartResult = { name: string; max_stress: number; max_displacement: number; yield_strength: number; safety: number; floating: boolean };
type ContactPair = { parts: [string, string]; kind: ContactKind; friction: number; area: number; pressing: number; slipping: number; normal_force: number; shear_force: number; max_pressure: number; press_fit?: boolean; interference?: number; mean_pressure: number };
type HeatSummary = { reference: number; min: number; max: number; thermal_stress: number | null; mechanical_stress: number | null };
type Peak = { at: V3; stress: number; coarse: number; cell: number; change: number; singular: boolean; at_support: boolean };
type Contacts = { faces: { cell: V3; dir: number; pressure: number; pair: number }[]; opened: [number, number, number, number][]; max_pressure: number; pairs: ContactPair[]; rounds: number; settled: boolean };
type Reaction = { name: string; force: V3; moment: V3; at: V3 };
type Result = NonNullable<StressView["result"]> & {
  yield_strength: number;
  safety: number;
  elements: number;
  iterations: number;
  notes: string[];
  parts?: PartResult[];
  contact?: Contacts;
  heat?: HeatSummary;
  peak?: Peak;
  max_stress_at?: V3;
  max_displacement_at?: V3;
  reactions?: Reaction[];
  bolts?: { part: string; preload: number; force: number; shortening: number }[];
  plastic?: { max_strain: number; at: V3; yielded: number; steps: number; settled: boolean; tangent: number };
  plastic_strain?: number[];
};
type Fatigue = { factors: number[]; min_factor: number; at: V3; life: number | null; strength: number; marin: V3; amplitude: number; mean: number; notes: string[] };
type Done = { kind: "done"; result: Result; material: string; key?: string; properties?: MaterialProps; fatigue?: Fatigue | null };
const KINDS: { value: ContactKind; label: string; title: string }[] = [
  { value: "frictionless", label: "Slides", title: "Frictionless: they push on each other, never pull, and slide freely" },
  { value: "friction", label: "Grips", title: "Friction: they push, never pull, and grip sideways up to μ × the push" },
  { value: "bonded", label: "Glued", title: "Bonded: they move together where they touch" },
];
type Shapes = {
  cell: number;
  origin: [number, number, number];
  cells: [number, number, number][];
  nodes: [number, number, number][];
  mass?: number;
  notes: string[];
};
type Modes = Shapes & { modes: { frequency: number; direction: string; amount: number[]; shape: [number, number, number][] }[] };
type Buckling = Shapes & { modes: { factor: number; direction: string; amount: number[]; shape: [number, number, number][] }[]; max_stress: number; yield_factor: number };
type Convergence = { rows: { resolution: number; cell: number; elements: number; max_stress: number; max_displacement: number; stress_change: number | null; displacement_change: number | null; seconds: number }[]; displacement_limit: number | null; converged: boolean; notes: string[] };
type Probe = { at: V3; stress: number[]; von_mises: number; principal: V3; displacement: V3; safety: number; plastic_strain?: number };
type Layer = "stress" | "contact" | "safety" | "movement" | "fatigue" | "plastic";

const DIRECTIONS: { value: Direction; label: string }[] = [
  { value: "into", label: "Into the face" },
  { value: "pressure", label: "Pressure (MPa)" },
  { value: "-z", label: "Down (−Z)" },
  { value: "+z", label: "Up (+Z)" },
  { value: "-x", label: "−X" },
  { value: "+x", label: "+X" },
  { value: "-y", label: "−Y" },
  { value: "+y", label: "+Y" },
];
const AXES: { value: string; label: string; v: V3 }[] = [
  { value: "-z", label: "−Z", v: [0, 0, -1] },
  { value: "+z", label: "+Z", v: [0, 0, 1] },
  { value: "-x", label: "−X", v: [-1, 0, 0] },
  { value: "+x", label: "+X", v: [1, 0, 0] },
  { value: "-y", label: "−Y", v: [0, -1, 0] },
  { value: "+y", label: "+Y", v: [0, 1, 0] },
];
const SURFACES = [
  { value: "polished", label: "Polished" },
  { value: "ground", label: "Ground" },
  { value: "machined", label: "Machined" },
  { value: "hot-rolled", label: "Hot-rolled" },
  { value: "forged", label: "Forged" },
];
/** What a load's direction means, flat face or round. */
const LOAD_TITLE =
  "Into the face: on a flat face, the force square to it. On a round face (a bore, a shaft, a fillet), the newtons spread evenly over its area, each bit pushing straight in, so pushes on opposite sides cancel. Pressure: the same push given in MPa (N/mm²), as in a pressed bore or a pipe. An axis: the whole force one way, spread over the face by area.";
const RESOLUTIONS = [
  { label: "Quick", value: 32 },
  { label: "Normal", value: 56 },
  { label: "Fine", value: 96 },
];

const n = (v: number, d = 1) => Number(v.toFixed(d)).toLocaleString("en-US");
const unit = (v: V3): V3 => {
  const l = Math.hypot(...v) || 1;
  return v.map((c) => c / l) as V3;
};
/** The axis a vector mostly runs along, as a label key. */
const axisOf = (v: V3) => AXES.find((a) => a.v.every((c, i) => Math.abs(c - unit(v)[i]) < 1e-6))?.value ?? "-z";

const emptySpec = (part: string | null): Spec => ({ part, supports: [], loads: [], holds: [], heats: [], convection: null, resolution: 56, parts: [], contacts: [], heat_stress: null, refine: true });
const tone = (safety: number) => (safety >= 2 ? "ok" : safety >= 1.2 ? "warn" : "bad");
const noFaces: Resolved = { supports: [], loads: [], holds: [], heats: [] };
/** The newer holds and loads, whose faces GET /study may return found again (each `{ face }`, or the face itself for `sliding`). */
type Extra = "sliding" | "elastic_supports" | "displacements" | "bearing_loads" | "remote_loads";
const EXTRAS: Extra[] = ["sliding", "elastic_supports", "displacements", "bearing_loads", "remote_loads"];
const keyOf = (p: Pick) => JSON.stringify([p.at, p.probe, p.normal, p.part ?? null, !!p.curved]);

export function StressStudy(props: {
  id: string;
  part: string | null;
  version: unknown;
  material: string | null;
  /** The part's bounds (part coordinates), to keep faces by where they sit. */
  bounds: { min: V3; max: V3 } | null;
  /** The shape's saved studies as stored (changes when anyone edits them). */
  saved: unknown;
  /** Every part of the assembly (empty for a single solid). */
  assembly: PartStats[];
  run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
  /** Asks for a face; `parts`: other parts whose faces may be picked too. */
  onPickFace: (purpose: string, take: (f: ProgramFace) => void, parts?: string[]) => void;
  onShow: (id: string, view: StressView | null) => void;
}) {
  const [spec, setSpec] = useState<Spec>(() => emptySpec(props.part));
  const [resolved, setResolved] = useState<Resolved>(noFaces);
  // Faces of the newer holds and loads, as picked here (the engine finds them again itself when it runs the saved study).
  const [picked, setPicked] = useState<Record<string, Face>>({});
  const [heat, setHeat] = useState<{ kind: "busy" } | { kind: "done"; temps: Temps } | { kind: "error"; text: string } | null>(null);
  // Which result the view shows: the last one run.
  const [showing, setShowing] = useState<"stress" | "modes" | "heat" | "buckle">("stress");
  const resolution = spec.resolution;
  const [state, setState] = useState<{ kind: "idle" | "busy" } | Done | { kind: "error"; text: string }>({ kind: "idle" });
  const [deform, setDeform] = useState(false);
  const [vib, setVib] = useState<{ kind: "busy" } | { kind: "done"; modes: Modes } | { kind: "error"; text: string } | null>(null);
  const [mode, setMode] = useState<number | null>(null);
  const [buckle, setBuckle] = useState<{ kind: "busy" } | { kind: "done"; b: Buckling } | { kind: "error"; text: string } | null>(null);
  const [shape, setShape] = useState<number | null>(null);
  const [conv, setConv] = useState<{ kind: "busy" } | { kind: "done"; c: Convergence } | { kind: "error"; text: string } | null>(null);
  // The parts that touch this one (null: still finding out), and which result layer shows.
  const [touching, setTouching] = useState<string[] | null>(null);
  const [layer, setLayer] = useState<Layer>("stress");
  const [section, setSection] = useState<{ axis: 0 | 1 | 2; at: number } | null>(null);
  const [probeAt, setProbeAt] = useState<V3 | null>(null);
  const [probe, setProbe] = useState<{ kind: "done"; p: Probe } | { kind: "error"; text: string } | null>(null);
  const [more, setMore] = useState(false);

  // The saved study, its faces found on the part as it is now.
  useEffect(() => {
    let live = true;
    fetch(projectUrl(`/study/${props.id}${props.part ? `?part=${encodeURIComponent(props.part)}` : ""}`))
      .then((r) => (r.ok ? r.json() : null))
      .then((out: ({ spec: Spec | null } & Partial<Resolved> & Partial<Record<Extra, ({ face: Face | null } | Face | null)[]>>) | null) => {
        if (!live || !out) return;
        const s: Spec = { ...emptySpec(props.part), ...(out.spec ?? {}) };
        setSpec(s);
        setResolved({ supports: out.supports ?? [], loads: out.loads ?? [], holds: out.holds ?? [], heats: out.heats ?? [] });
        // The newer holds and loads as the engine finds them now, when it says.
        const found: Record<string, Face> = {};
        for (const list of EXTRAS) {
          const picks: Pick[] = list === "sliding" ? (s.sliding ?? []) : ((s[list] ?? []) as { face: Pick }[]).map((x) => x.face);
          (out[list] ?? []).forEach((x, i) => {
            const f = x && "triangles" in x ? x : x?.face;
            if (f && picks[i]) found[keyOf(picks[i])] = f;
          });
        }
        if (Object.keys(found).length) setPicked((m) => ({ ...m, ...found }));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [props.id, props.part, props.version, props.saved]);
  // The other parts it touches: the ones that can press on it in the study.
  const several = !!props.part && props.assembly.length > 1;
  useEffect(() => {
    if (!several || !props.part) return setTouching([]);
    let live = true;
    setTouching(null);
    fetch(projectUrl(`/touching/${props.id}?part=${encodeURIComponent(props.part)}`))
      .then((r) => (r.ok ? r.json() : { parts: [] }))
      .then((out: { parts: string[] }) => live && setTouching(out.parts))
      .catch(() => live && setTouching([]));
    return () => {
      live = false;
    };
  }, [props.id, props.part, props.version, several]);
  // Another part, or a new version of it: earlier results no longer apply.
  useEffect(() => {
    setState({ kind: "idle" });
    setVib(null);
    setMode(null);
    setHeat(null);
    setBuckle(null);
    setShape(null);
    setConv(null);
    setProbe(null);
  }, [props.id, props.part, props.version]);

  const supports = resolved.supports.filter((f): f is Face => !!f);
  const loads = resolved.loads.filter((l): l is { face: Face; force: V3; pressure?: number } => !!l.face && !!l.force);
  /** A picked face kept by where it sits on its part (this one, or another studied with it). */
  const pickOf = (f: ProgramFace): Pick => {
    const other = f.part && f.part !== props.part ? props.assembly.find((p) => p.name === f.part) : undefined;
    const b = other ? { min: other.bounds_min, max: other.bounds_max } : props.bounds;
    const frac = (p: V3): V3 => (b ? ([0, 1, 2].map((k) => (b.max[k] - b.min[k] > 1e-9 ? (p[k] - b.min[k]) / (b.max[k] - b.min[k]) : 0.5)) as V3) : [0.5, 0.5, 0.5]);
    const t = f.triangles;
    const probe: V3 = [(t[0] + t[3] + t[6]) / 3, (t[1] + t[4] + t[7]) / 3, (t[2] + t[5] + t[8]) / 3];
    return { at: frac(f.centroid), probe: frac(probe), normal: f.normal, ...(other ? { part: other.name } : {}), ...(f.curved ? { curved: true } : {}) };
  };
  /** A pick, remembering the face it was picked from (for the newer holds and loads). */
  const remember = (f: ProgramFace): Pick => {
    const p = pickOf(f);
    const other = f.part && f.part !== props.part ? f.part : null;
    setPicked((m) => ({ ...m, [keyOf(p)]: { point: f.centroid, normal: f.normal, triangles: Array.from(f.triangles), part: other, curved: !!f.curved } }));
    return p;
  };
  const faceOf = (p: Pick): Face | undefined => picked[keyOf(p)];
  /** Asks for a face on this part or any studied with it. */
  const pickFace = (purpose: string, take: (f: ProgramFace) => void) => props.onPickFace(purpose, take, spec.parts);
  /** The contact between this part and another (frictionless unless set). */
  const contactWith = (name: string): ContactSpec => spec.contacts.find((c) => c.parts.includes(name) && c.parts.includes(props.part ?? "")) ?? { parts: [props.part ?? "", name], kind: "frictionless", friction: 0.3 };
  const setContact = (name: string, change: Partial<ContactSpec>) => {
    const next = { ...contactWith(name), ...change };
    save({ ...spec, contacts: [...spec.contacts.filter((c) => !(c.parts.includes(name) && c.parts.includes(props.part ?? ""))), next] });
  };
  /** Takes another part into the study, or leaves it out (with its supports and loads). */
  const include = (name: string, on: boolean) => {
    if (on) return save({ ...spec, parts: [...spec.parts, name] });
    const mine = (p: Pick) => p.part !== name;
    save({
      ...spec,
      parts: spec.parts.filter((p) => p !== name),
      contacts: spec.contacts.filter((c) => !c.parts.includes(name)),
      supports: spec.supports.filter(mine),
      loads: spec.loads.filter((l) => mine(l.face)),
    });
  };
  /** Keeps a change with the project (it syncs to teammates and undoes like any edit). */
  const save = (next: Spec) => {
    setSpec(next);
    changed();
    return props.run("shape.set_study", { target: props.id, study: { ...next, part: props.part } });
  };

  const sliding = spec.sliding ?? [];
  const springs = spec.elastic_supports ?? [];
  const moved = spec.displacements ?? [];
  const bearings = spec.bearing_loads ?? [];
  const moments = spec.remote_loads ?? [];
  // Every newer face known here: the study can be sent as it is; else the engine runs the saved one.
  const extrasKnown = [...sliding, ...springs.map((s) => s.face), ...moved.map((d) => d.face), ...bearings.map((b) => b.face), ...moments.map((m) => m.face)].every((p) => !!faceOf(p));
  const anyHold = supports.length + sliding.length + springs.length + moved.length > 0;
  const anyLoad = loads.length + bearings.length + moments.length > 0 || !!spec.gravity || !!spec.spin || moved.some((d) => d.by.some((v) => v)) ;

  const r = state.kind === "done" ? state.result : null;
  const fatigue = state.kind === "done" ? state.fatigue : null;
  const view = useMemo((): StressView => {
    const thermalFaces: StressView["faces"] = [
      ...resolved.holds.flatMap((h) => (h.face ? [{ kind: "hold" as const, triangles: h.face.triangles, point: h.face.point }] : [])),
      ...resolved.heats.flatMap((h) => (h.face ? [{ kind: "heat" as const, triangles: h.face.triangles, point: h.face.point }] : [])),
    ];
    if (showing === "heat" && heat?.kind === "done") {
      const t = heat.temps;
      return { faces: thermalFaces, section, result: { cell: t.cell, origin: t.origin, cells: t.cells.map(([c, v]) => [c, v - t.min] as [V3, number]), displacements: [], max_stress: Math.max(t.max - t.min, 1e-9), max_displacement: 1 } };
    }
    const extraHolds = [...sliding, ...springs.map((s) => s.face), ...moved.map((d) => d.face)].flatMap((p) => {
      const f = faceOf(p);
      return f ? [{ kind: "support" as const, triangles: f.triangles, point: f.point }] : [];
    });
    const extraLoads = [
      ...bearings.flatMap((b) => {
        const f = faceOf(b.face);
        return f ? [{ kind: "load" as const, triangles: f.triangles, point: f.point, force: b.force }] : [];
      }),
      ...moments.flatMap((m) => {
        const f = faceOf(m.face);
        return f ? [{ kind: "load" as const, triangles: f.triangles, point: f.point }] : [];
      }),
    ];
    const faces: StressView["faces"] = [
      ...supports.map((f) => ({ kind: "support" as const, triangles: f.triangles, point: f.point })),
      ...extraHolds,
      ...loads.map((l) => ({ kind: "load" as const, triangles: l.face.triangles, point: l.face.point, force: l.force, pressure: !!l.pressure })),
      ...extraLoads,
      ...thermalFaces,
    ];
    // A vibration or buckling shape, when one is picked, shows swinging; else the result map.
    const swing = (s: Shapes, m: { amount: number[]; shape: V3[] }): StressView => ({
      faces: faces.filter((f) => f.kind === "support"),
      oscillate: true,
      section,
      result: {
        cell: s.cell,
        origin: s.origin,
        cells: s.cells.map((c, i) => [c, m.amount[i]] as [V3, number]),
        displacements: s.nodes.map((c, i) => [c, m.shape[i]] as [V3, V3]),
        max_stress: 1,
        max_displacement: 1,
      },
    });
    if (showing === "buckle" && buckle?.kind === "done" && shape !== null && buckle.b.modes[shape]) return swing(buckle.b, buckle.b.modes[shape]);
    if (vib?.kind === "done" && mode !== null && vib.modes.modes[mode]) return swing(vib.modes, vib.modes.modes[mode]);
    if (!r) return { faces };
    const contact = r.contact ? { faces: r.contact.faces, opened: r.contact.opened, max_pressure: r.contact.max_pressure } : undefined;
    const peak = r.peak && layer === "stress" ? { at: r.peak.at, singular: r.peak.singular } : undefined;
    const markers = [...(r.max_stress_at && layer !== "movement" ? [{ at: r.max_stress_at, color: "#e0452f", label: "largest stress" }] : []), ...(r.max_displacement_at && r.max_displacement > 0 ? [{ at: r.max_displacement_at, color: "#2b59c3", label: "largest movement" }] : [])];
    const result = layered(r, layer, fatigue);
    return { faces, result, deform: deform ? 1 : 0, contact, layer: contact && layer === "contact" ? "contact" : "stress", peak, markers, section };
  }, [resolved, state, deform, vib, mode, heat, showing, layer, buckle, shape, section, picked]); // eslint-disable-line react-hooks/exhaustive-deps
  const { id, onShow } = props;
  useEffect(() => {
    onShow(id, view.faces.length || view.result ? view : null);
  }, [id, view, onShow]);
  useEffect(() => () => onShow(id, null), [id, onShow]);

  const holds = resolved.holds.map((h, i) => (h.face ? { face: h.face, temperature: spec.holds[i]?.temperature ?? 20 } : null)).filter((x): x is { face: Face; temperature: number } => !!x);
  const heats = resolved.heats.map((h, i) => (h.face ? { face: h.face, watts: spec.heats[i]?.watts ?? 0 } : null)).filter((x): x is { face: Face; watts: number } => !!x);
  const heatOn = !!spec.heat_stress && (holds.length > 0 || !!(spec.convection && spec.convection.h > 0));
  const pressing = spec.contacts.some((c) => c.press_fit && spec.parts.some((p) => c.parts.includes(p)));

  /** The study as the engine takes it: the faces as found now. */
  const body = (extra: Record<string, unknown> = {}) => ({
    supports,
    loads: loads.map((l) => ({ face: l.face, force: l.force, pressure: l.pressure ?? 0 })),
    sliding: sliding.map(faceOf).filter(Boolean),
    elastic_supports: springs.flatMap((s) => (faceOf(s.face) ? [{ face: faceOf(s.face), stiffness: s.stiffness, normal_only: s.normal_only }] : [])),
    displacements: moved.flatMap((d) => (faceOf(d.face) ? [{ face: faceOf(d.face), by: d.by }] : [])),
    bearing_loads: bearings.flatMap((b) => (faceOf(b.face) ? [{ face: faceOf(b.face), force: b.force }] : [])),
    remote_loads: moments.flatMap((m) => (faceOf(m.face) ? [{ face: faceOf(m.face), point: m.point, force: m.force, moment: m.moment }] : [])),
    gravity: spec.gravity ?? null,
    spin: spec.spin ?? null,
    plastic: !!spec.plastic,
    resolution,
    part: props.part,
    parts: spec.parts,
    contacts: spec.contacts.filter((c) => c.parts.every((p) => p === props.part || spec.parts.includes(p))),
    refine: spec.refine !== false && !spec.parts.length,
    heat: heatOn ? { reference: spec.heat_stress!.reference, thermal: { holds, heats, convection: spec.convection && spec.convection.h > 0 ? spec.convection : null } } : null,
    ...extra,
  });
  const post = (path: string, extra: Record<string, unknown> = {}) => fetch(projectUrl(path), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body(extra)) });

  const run = async () => {
    setState({ kind: "busy" });
    setProbe(null);
    try {
      // Newer faces picked in another session are found again by the engine from the saved study.
      const r = extrasKnown
        ? await post(`/stress/${props.id}`, { fatigue: spec.fatigue ?? null })
        : await fetch(projectUrl(`/simulate/${props.id}?kind=stress${props.part ? `&part=${encodeURIComponent(props.part)}` : ""}`), { method: "POST" });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Result; material: string; key?: string; properties?: MaterialProps; fatigue?: Fatigue | null };
      setState({ kind: "done", result: out.result, material: out.material, key: out.key, properties: out.properties, fatigue: out.fatigue });
      setShowing("stress");
      setMode(null);
      setShape(null);
      if (out.result.max_stress_at) setProbeAt(out.result.max_stress_at);
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };

  function changed() {
    setState((s) => (s.kind === "done" ? { kind: "idle" } : s));
    setVib(null);
    setMode(null);
    setHeat(null);
    setBuckle(null);
    setShape(null);
    setConv(null);
    setProbe(null);
  }
  const runHeat = async () => {
    setHeat({ kind: "busy" });
    setMode(null);
    try {
      const r = await fetch(projectUrl(`/thermal/${props.id}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ holds, heats, convection: spec.convection && spec.convection.h > 0 ? spec.convection : null, resolution: Math.min(resolution, 72), part: props.part }),
      });
      if (!r.ok) return setHeat({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Temps };
      setHeat({ kind: "done", temps: out.result });
      setShowing("heat");
    } catch {
      setHeat({ kind: "error", text: "The engine did not answer" });
    }
  };
  const ring = async () => {
    setVib({ kind: "busy" });
    setMode(null);
    try {
      const r = await post(`/modes/${props.id}`, { resolution: Math.min(resolution, 64), count: 4, prestress: !!spec.prestress });
      if (!r.ok) return setVib({ kind: "error", text: await r.text() });
      const modes = (await r.json()) as Modes;
      setVib({ kind: "done", modes });
      setMode(0);
      setShowing("modes");
    } catch {
      setVib({ kind: "error", text: "The engine did not answer" });
    }
  };
  const runBuckle = async () => {
    setBuckle({ kind: "busy" });
    setShape(null);
    try {
      const r = await post(`/buckle/${props.id}`, { resolution: Math.min(resolution, 64), count: 3 });
      if (!r.ok) return setBuckle({ kind: "error", text: await r.text() });
      const b = (await r.json()) as Buckling;
      setBuckle({ kind: "done", b });
      setMode(null);
      setShape(b.modes.length ? 0 : null);
      setShowing("buckle");
    } catch {
      setBuckle({ kind: "error", text: "The engine did not answer" });
    }
  };
  const runConverge = async () => {
    setConv({ kind: "busy" });
    try {
      const r = await post(`/converge/${props.id}`, { refine: false });
      if (!r.ok) return setConv({ kind: "error", text: await r.text() });
      setConv({ kind: "done", c: (await r.json()) as Convergence });
    } catch {
      setConv({ kind: "error", text: "The engine did not answer" });
    }
  };
  const exportDeck = async () => {
    const r = await post(`/inp/${props.id}`, { kind: "static" });
    if (!r.ok) return setState((s) => (s.kind === "done" ? s : { kind: "error", text: "The deck could not be written" }));
    const url = URL.createObjectURL(new Blob([await r.text()], { type: "text/plain" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `study-${props.part ?? props.id}.inp`;
    a.click();
    URL.revokeObjectURL(url);
  };
  const runProbe = async () => {
    if (state.kind !== "done" || !state.key || !probeAt) return;
    try {
      const res = await fetch(projectUrl(`/results/${state.key}/probe?at=${probeAt.join(",")}`));
      if (!res.ok) return setProbe({ kind: "error", text: await res.text() });
      setProbe({ kind: "done", p: (await res.json()) as Probe });
    } catch {
      setProbe({ kind: "error", text: "The engine did not answer" });
    }
  };

  const verdict = r ? tone(r.safety) : "";
  const onWhat = (p: Pick) => (p.part ? ` on ${p.part}` : "");
  const lost = (p: Pick) => !faceOf(p);
  const num = (value: number, onSet: (v: number) => void, label: string, key: string) => (
    <input
      defaultValue={String(value)}
      key={key}
      inputMode="decimal"
      aria-label={label}
      onBlur={(e) => {
        const v = Number(e.target.value);
        if (Number.isFinite(v) && v !== value) onSet(v);
      }}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
  const remove = <T,>(list: T[], i: number) => list.filter((_, j) => j !== i);
  const replace = <T,>(list: T[], i: number, x: T) => list.map((y, j) => (j === i ? x : y));
  const bounds = props.bounds;
  return (
    <section className="props-section stress-study">
      <header>
        <h3>Stress</h3>
        <span className="section-note">{props.material ? `${props.material} · typical properties unless set` : "Give the part a material first"}</span>
      </header>
      <MaterialProperties material={props.material} version={props.version} run={props.run} onChange={changed} />
      <div className="stress-list">
        {spec.supports.map((_, i) => (
          <div key={`s${i}`} className={`stress-item support ${resolved.supports[i] === null ? "lost" : ""}`}>
            <span className="stress-dot" />
            {resolved.supports[i] === null ? "Face no longer found: pick it again" : `Held at ${spec.supports[i].curved ? "round " : ""}face ${i + 1}${onWhat(spec.supports[i])}`}
            <button type="button" aria-label="Remove this support" onClick={() => save({ ...spec, supports: spec.supports.filter((_, j) => j !== i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {sliding.map((p, i) => (
          <div key={`sl${i}`} className="stress-item support" title="Held square to the face only: it slides along it (a symmetry plane, a frictionless support, a bearing seat)">
            <span className="stress-dot" />
            Slides on {p.curved ? "round " : ""}face{onWhat(p)}
            {lost(p) && <small className="stress-on">found when run</small>}
            <button type="button" aria-label="Remove this sliding face" onClick={() => save({ ...spec, sliding: remove(sliding, i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {springs.map((s, i) => (
          <div key={`sp${i}`} className="stress-item support" title="Resting on springs: N/mm of push per mm² of face for each mm it gives (a rubber foot about 0.5–5, a foundation more)">
            <span className="stress-dot" />
            On springs
            {num(s.stiffness, (v) => save({ ...spec, elastic_supports: replace(springs, i, { ...s, stiffness: v }) }), "Spring stiffness in newtons per millimetre per square millimetre", `sp${i}:${s.stiffness}`)}
            N/mm³
            <label className="contact-mu" title="Only square to the face: it still slides along it">
              <input type="checkbox" checked={s.normal_only} onChange={(e) => save({ ...spec, elastic_supports: replace(springs, i, { ...s, normal_only: e.target.checked }) })} /> square only
            </label>
            <button type="button" aria-label="Remove these springs" onClick={() => save({ ...spec, elastic_supports: remove(springs, i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {moved.map((d, i) => (
          <div key={`mv${i}`} className="stress-item support moved" title="The face moved by these amounts; leave one blank to let it move freely that way">
            <span className="stress-dot" />
            Moved by
            {(["x", "y", "z"] as const).map((axis, k) => (
              <label key={axis} className="moved-axis">
                {axis}
                <input
                  defaultValue={d.by[k] == null ? "" : String(d.by[k])}
                  key={`mv${i}${k}:${d.by[k]}`}
                  inputMode="decimal"
                  placeholder="free"
                  aria-label={`Movement along ${axis} in millimetres (blank: free)`}
                  onBlur={(e) => {
                    const raw = e.target.value.trim();
                    const v = raw === "" ? null : Number(raw);
                    if (v !== null && !Number.isFinite(v)) return;
                    if (v !== d.by[k]) save({ ...spec, displacements: replace(moved, i, { ...d, by: d.by.map((x, j) => (j === k ? v : x)) }) });
                  }}
                  onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                />
              </label>
            ))}
            mm
            <button type="button" aria-label="Remove this movement" onClick={() => save({ ...spec, displacements: remove(moved, i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {spec.loads.map((l, i) => (
          <div key={`l${i}`} className={`stress-item load ${resolved.loads[i]?.face === null ? "lost" : ""}`}>
            <span className="stress-dot" />
            <input
              defaultValue={String(l.magnitude)}
              key={`${i}:${l.magnitude}`}
              inputMode="decimal"
              aria-label={l.direction === "pressure" ? "Pressure in megapascals" : "Force in newtons"}
              onBlur={(e) => {
                const v = Number(e.target.value);
                if (Number.isFinite(v) && v !== l.magnitude) save({ ...spec, loads: spec.loads.map((x, j) => (j === i ? { ...x, magnitude: v } : x)) });
              }}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            {l.direction === "pressure" ? "MPa" : "N"}
            <select value={l.direction} aria-label="Direction" title={LOAD_TITLE} onChange={(e) => save({ ...spec, loads: spec.loads.map((x, j) => (j === i ? { ...x, direction: e.target.value as Direction } : x)) })}>
              {DIRECTIONS.map((d) => (
                <option key={d.value} value={d.value}>
                  {d.label}
                </option>
              ))}
            </select>
            {l.face.part && <span className="stress-on">{l.face.part}</span>}
            <button type="button" aria-label="Remove this load" onClick={() => save({ ...spec, loads: spec.loads.filter((_, j) => j !== i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {bearings.map((b, i) => {
          const size = Math.hypot(...b.force);
          return (
            <div key={`br${i}`} className="stress-item load" title="A shaft pressing in a bore (or a bearing on a shaft): the force spread over the half of the round face it pushes into, most where it pushes squarely">
              <span className="stress-dot" />
              Bearing
              {num(size, (v) => save({ ...spec, bearing_loads: replace(bearings, i, { ...b, force: unit(b.force).map((c) => c * v) as V3 }) }), "Bearing force in newtons", `br${i}:${size}`)}N
              <select value={axisOf(b.force)} aria-label="Which way the shaft pushes" onChange={(e) => save({ ...spec, bearing_loads: replace(bearings, i, { ...b, force: AXES.find((a) => a.value === e.target.value)!.v.map((c) => c * size) as V3 }) })}>
                {AXES.map((a) => (
                  <option key={a.value} value={a.value}>
                    {a.label}
                  </option>
                ))}
              </select>
              <button type="button" aria-label="Remove this bearing load" onClick={() => save({ ...spec, bearing_loads: remove(bearings, i) })}>
                <Icon name="close" />
              </button>
            </div>
          );
        })}
        {moments.map((m, i) => {
          const size = Math.hypot(...m.moment) / 1000;
          return (
            <div key={`mo${i}`} className="stress-item load" title="A moment (a motor's torque, a lever's twist) about the face's middle, carried to the face as by a rigid bracket">
              <span className="stress-dot" />
              Moment
              {num(size, (v) => save({ ...spec, remote_loads: replace(moments, i, { ...m, moment: unit(m.moment).map((c) => c * v * 1000) as V3 }) }), "Moment in newton-metres", `mo${i}:${size}`)}N·m about
              <select value={axisOf(m.moment)} aria-label="The axis it turns about" onChange={(e) => save({ ...spec, remote_loads: replace(moments, i, { ...m, moment: AXES.find((a) => a.value === e.target.value)!.v.map((c) => c * size * 1000) as V3 }) })}>
                {AXES.map((a) => (
                  <option key={a.value} value={a.value}>
                    {a.label}
                  </option>
                ))}
              </select>
              <button type="button" aria-label="Remove this moment" onClick={() => save({ ...spec, remote_loads: remove(moments, i) })}>
                <Icon name="close" />
              </button>
            </div>
          );
        })}
        {spec.spin && (
          <div className="stress-item load" title="Spinning about an axis through the part's middle: every bit pulled outward by ρ·ω²·r">
            <span className="stress-dot" />
            Spinning at
            {num(spec.spin.rpm, (v) => save({ ...spec, spin: { ...spec.spin!, rpm: v } }), "Speed in revolutions per minute", `spin:${spec.spin.rpm}`)}
            rpm about
            <select value={axisOf(spec.spin.axis).replace("-", "+")} aria-label="The axis it spins about" onChange={(e) => save({ ...spec, spin: { ...spec.spin!, axis: AXES.find((a) => a.value === e.target.value)!.v } })}>
              {AXES.filter((a) => a.value.startsWith("+")).map((a) => (
                <option key={a.value} value={a.value}>
                  {a.label.replace("+", "")}
                </option>
              ))}
            </select>
            <button type="button" aria-label="Stop the spinning" onClick={() => save({ ...spec, spin: null })}>
              <Icon name="close" />
            </button>
          </div>
        )}
      </div>
      <div className="stress-add">
        <button type="button" className="secondary" onClick={() => pickFace(spec.parts.length ? "Click the face it is held by, flat or round (on this part or one studied with it)" : "Click the face the part is held by, flat or round", (f) => save({ ...spec, supports: [...spec.supports, pickOf(f)] }))}>
          <Icon name="plus" /> Support
        </button>
        <button type="button" className="secondary" onClick={() => pickFace(spec.parts.length ? "Click the face the force pushes on, flat or round (on this part or one studied with it)" : "Click the face the force pushes on, flat or round", (f) => save({ ...spec, loads: [...spec.loads, { face: pickOf(f), magnitude: 100, direction: "into" }] }))}>
          <Icon name="plus" /> Load
        </button>
        <button type="button" className="secondary" aria-expanded={more} onClick={() => setMore(!more)} title="Sliding faces, springs, set movements, bearings, moments, gravity, spinning, yielding">
          <Icon name="sliders" /> More
        </button>
        {!spec.parts.length && (
          <label className="xray-toggle" title="Re-solve the most stressed spot on cubes up to 8× finer and read the stress at the surface itself">
            <input type="checkbox" checked={spec.refine !== false} onChange={(e) => save({ ...spec, refine: e.target.checked })} /> Sharpen the peak
          </label>
        )}
        <div className="segmented" role="group" aria-label="Resolution">
          {RESOLUTIONS.map((x) => (
            <button key={x.value} type="button" className={x.value === resolution ? "active" : ""} aria-pressed={x.value === resolution} onClick={() => save({ ...spec, resolution: x.value })}>
              {x.label}
            </button>
          ))}
        </div>
      </div>
      {more && (
        <div className="stress-more">
          <div className="stress-add">
            <button type="button" className="secondary" title="Held square to the face only, free to slide along it: a symmetry plane, a frictionless support, a bearing seat (round)" onClick={() => pickFace("Click the face that slides (held square to itself only)", (f) => save({ ...spec, sliding: [...sliding, remember(f)] }))}>
              <Icon name="plus" /> Slides
            </button>
            <button type="button" className="secondary" title="A face resting on springs: a rubber foot, a soft mount, a foundation" onClick={() => pickFace("Click the face that rests on springs", (f) => save({ ...spec, elastic_supports: [...springs, { face: remember(f), stiffness: 1, normal_only: true }] }))}>
              <Icon name="plus" /> Springs
            </button>
            <button type="button" className="secondary" title="A face moved by set amounts (a press fit's squeeze, a part pulled into place)" onClick={() => pickFace("Click the face that is moved", (f) => save({ ...spec, displacements: [...moved, { face: remember(f), by: [null, null, 0] }] }))}>
              <Icon name="plus" /> Moved by
            </button>
            <button type="button" className="secondary" title="A shaft pushing in a bore (or a bearing on a shaft): pick the round face" onClick={() => pickFace("Click the round face the shaft or bearing pushes on", (f) => save({ ...spec, bearing_loads: [...bearings, { face: remember(f), force: [0, 0, -100] }] }))}>
              <Icon name="plus" /> Bearing
            </button>
            <button type="button" className="secondary" title="A twist about the face's middle (a motor's torque)" onClick={() => pickFace("Click the face the moment turns", (f) => save({ ...spec, remote_loads: [...moments, { face: remember(f), point: f.centroid, force: [0, 0, 0], moment: [0, 0, 1000] }] }))}>
              <Icon name="plus" /> Moment
            </button>
          </div>
          <div className="stress-add">
            <label className="xray-toggle" title="Its own weight pulls it down (−Z, 9.81 m/s²)">
              <input type="checkbox" checked={!!spec.gravity} onChange={(e) => save({ ...spec, gravity: e.target.checked ? [0, 0, -9.81] : null })} /> Its own weight
            </label>
            <label className="xray-toggle" title="Spinning about an axis through the part's middle (a rotor, a pulley, a fan)">
              <input
                type="checkbox"
                checked={!!spec.spin}
                onChange={(e) => {
                  const mid: V3 = bounds ? ([0, 1, 2].map((k) => (bounds.min[k] + bounds.max[k]) / 2) as V3) : [0, 0, 0];
                  save({ ...spec, spin: e.target.checked ? { at: mid, axis: [0, 0, 1], rpm: 3000 } : null });
                }}
              />{" "}
              Spinning
            </label>
            <label className="xray-toggle" title="Let the material yield: bilinear plasticity (von Mises, hardening at the material's tangent modulus), the loads in ten steps. For one part on its own.">
              <input type="checkbox" checked={!!spec.plastic} onChange={(e) => save({ ...spec, plastic: e.target.checked })} /> Let it yield
            </label>
            <label className="xray-toggle" title="Fatigue: the loads come and go; the life and the safety at the wanted life">
              <input type="checkbox" checked={!!spec.fatigue} onChange={(e) => save({ ...spec, fatigue: e.target.checked ? { ratio: -1, cycles: 1e6, surface: "machined", reliability: 0.5 } : null })} /> Fatigue
            </label>
          </div>
          {spec.fatigue && (
            <div className="stress-item fatigue-spec">
              Loads go from
              {num(spec.fatigue.ratio, (v) => save({ ...spec, fatigue: { ...spec.fatigue!, ratio: v } }), "Smallest load over largest in each cycle (−1 fully reversed, 0 on and off)", `fr:${spec.fatigue.ratio}`)}
              × to full,
              {num(spec.fatigue.cycles, (v) => save({ ...spec, fatigue: { ...spec.fatigue!, cycles: v } }), "Life wanted in cycles", `fc:${spec.fatigue.cycles}`)}
              cycles,
              <select value={spec.fatigue.surface} aria-label="Surface finish" onChange={(e) => save({ ...spec, fatigue: { ...spec.fatigue!, surface: e.target.value } })}>
                {SURFACES.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
      )}
      {several && (
        <div className="contact-block">
          <h4>Parts touching it</h4>
          {touching === null && <p className="motion-note">Finding the parts that touch it…</p>}
          {touching?.length === 0 && <p className="motion-note">No other part touches it: it is studied on its own.</p>}
          {touching && [...new Set([...touching, ...spec.parts])].map((name) => {
            const on = spec.parts.includes(name);
            const c = contactWith(name);
            const material = props.assembly.find((p) => p.name === name)?.material;
            return (
              <div key={name} className={`contact-row ${on ? "on" : ""}`}>
                <label title={material ? `${material}: its own stiffness` : "Give it a material to study it"}>
                  <input type="checkbox" checked={on} disabled={!material && !on} onChange={(e) => include(name, e.target.checked)} />
                  <span>{name}</span>
                  {!material && <small>no material</small>}
                </label>
                {on && (
                  <div className="segmented" role="group" aria-label={`How ${name} acts where it touches`}>
                    {KINDS.map((k) => (
                      <button key={k.value} type="button" title={k.title} className={c.kind === k.value ? "active" : ""} aria-pressed={c.kind === k.value} onClick={() => setContact(name, { kind: k.value })}>
                        {k.label}
                      </button>
                    ))}
                  </div>
                )}
                {on && c.kind !== "bonded" && (
                  <label className="contact-mu" title="Press fit: where the parts overlap as drawn, the overlap is an interference squeezed out, so they start pressed together">
                    <input type="checkbox" checked={!!c.press_fit} onChange={(e) => setContact(name, { press_fit: e.target.checked })} /> Press fit
                  </label>
                )}
                {on && c.kind === "friction" && (
                  <label className="contact-mu" title="Coefficient of friction: about 0.15 oiled steel, 0.5 dry steel, 0.6 rubber on concrete">
                    μ
                    <input
                      defaultValue={String(c.friction)}
                      key={`${name}:${c.friction}`}
                      inputMode="decimal"
                      aria-label="Coefficient of friction"
                      onBlur={(e) => {
                        const v = Number(e.target.value);
                        if (Number.isFinite(v) && v >= 0 && v !== c.friction) setContact(name, { friction: v });
                      }}
                      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    />
                  </label>
                )}
              </div>
            );
          })}
          {spec.parts.length > 0 && <p className="motion-note">They press on it where they touch, never pull (unless glued). Supports and loads can go on them too; a part with no support of its own is held only by its contacts.</p>}
        </div>
      )}
      <div className="stress-runs">
        <button type="button" className="primary stress-run" disabled={!anyHold || (!anyLoad && !heatOn && !pressing) || state.kind === "busy" || !props.material} onClick={run} title="How hard the loads work the material, and how far it bends">
          <Icon name="weight" /> {state.kind === "busy" ? "Solving…" : "Stress"}
        </button>
        <button type="button" className="secondary stress-run" disabled={!anyHold || vib?.kind === "busy" || !props.material} onClick={ring} title="The speeds it wants to vibrate at, held at its supports (loads not needed, unless prestressed)">
          <Icon name="refresh" /> {vib?.kind === "busy" ? "Finding…" : "Vibration"}
        </button>
        <button type="button" className="secondary stress-run" disabled={!anyHold || !anyLoad || buckle?.kind === "busy" || !props.material || spec.parts.length > 0} onClick={runBuckle} title="How many times the loads it takes to buckle it (a slender strut, a thin wall)">
          <Icon name="collide" /> {buckle?.kind === "busy" ? "Finding…" : "Buckling"}
        </button>
        <button type="button" className="secondary stress-run" disabled={!anyHold || !anyLoad || conv?.kind === "busy" || !props.material} onClick={runConverge} title="The study on ever finer cubes: whether the answer has settled">
          <Icon name="grid" /> {conv?.kind === "busy" ? "Refining…" : "Converge"}
        </button>
      </div>
      {!extrasKnown && <p className="motion-note warn">Some sliding, spring, bearing or moment faces were picked in another session: Stress finds them again itself; for vibration, buckling and the rest, pick them again here.</p>}
      <label className="xray-toggle" title="Vibration with the loads applied: their stress stiffens the part where it pulls and softens it where it pushes">
        <input type="checkbox" checked={!!spec.prestress} onChange={(e) => save({ ...spec, prestress: e.target.checked })} /> Vibration under the loads (prestressed)
      </label>
      {vib?.kind === "error" && <p className="print-note bad">{vib.text}</p>}
      {vib?.kind === "done" && (
        <div className="mode-list">
          {vib.modes.modes.map((m, i) => (
            <button key={i} type="button" className={`mode-row ${mode === i && showing === "modes" ? "active" : ""}`} onClick={() => {
              setMode(mode === i ? null : i);
              setShowing("modes");
            }} aria-pressed={mode === i}>
              <span>Mode {i + 1}</span>
              <b>{n(m.frequency, m.frequency < 100 ? 1 : 0)} Hz</b>
              <small>
                {n(m.frequency * 60, 0)} rpm · mostly {m.direction.toUpperCase()}
              </small>
            </button>
          ))}
          <p className="motion-note">Avoid running a motor or engine near these speeds: the part would shake itself. {vib.modes.notes.join(" ")}</p>
        </div>
      )}
      {buckle?.kind === "error" && <p className="print-note bad">{buckle.text}</p>}
      {buckle?.kind === "done" && (
        <div className="mode-list">
          {buckle.b.modes.map((m, i) => (
            <button key={i} type="button" className={`mode-row ${shape === i && showing === "buckle" ? "active" : ""}`} onClick={() => {
              setShape(shape === i ? null : i);
              setMode(null);
              setShowing("buckle");
            }} aria-pressed={shape === i}>
              <span>Buckle {i + 1}</span>
              <b className={tone(m.factor / 1.5)}>×{n(m.factor, 2)}</b>
              <small>the loads · bows mostly {m.direction.toUpperCase()}</small>
            </button>
          ))}
          <p className="motion-note">
            {buckle.b.modes.length ? `It buckles at ${n(buckle.b.modes[0].factor, 2)} × the loads; it yields at ${n(buckle.b.yield_factor, 2)} ×. ` : ""}
            {buckle.b.notes.join(" ")}
          </p>
        </div>
      )}
      {conv?.kind === "error" && <p className="print-note bad">{conv.text}</p>}
      {conv?.kind === "done" && (
        <div className="converge">
          <table>
            <thead>
              <tr>
                <th>Cubes</th>
                <th>Largest stress</th>
                <th>Moves</th>
              </tr>
            </thead>
            <tbody>
              {conv.c.rows.map((row) => (
                <tr key={row.resolution}>
                  <td>
                    {row.elements.toLocaleString("en-US")} <small>of {n(row.cell, 2)} mm</small>
                  </td>
                  <td>
                    {n(row.max_stress, 1)} MPa {row.stress_change != null && <small>{n(row.stress_change * 100, 1)}%</small>}
                  </td>
                  <td>
                    {n(row.max_displacement, 4)} mm {row.displacement_change != null && <small>{n(row.displacement_change * 100, 1)}%</small>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className={`motion-note ${conv.c.converged ? "" : "warn"}`}>
            {conv.c.displacement_limit != null ? `Movement on infinitely fine cubes: about ${n(conv.c.displacement_limit, 4)} mm. ` : ""}
            {conv.c.notes.join(" ")}
          </p>
        </div>
      )}
      {state.kind === "error" && <p className="print-note bad">{state.text}</p>}
      {r && state.kind === "done" && (
        <div className="stress-result">
          <div className={`stress-verdict ${verdict}`}>
            <b>Safety ×{n(r.safety, 2)}</b> against yield ({n(r.yield_strength, 0)} MPa){r.parts && r.parts.length > 0 && ` · ${r.parts.reduce((a, b) => (b.safety < a.safety ? b : a)).name}`}
            {fatigue && (
              <>
                {" "}
                · <b className={tone(fatigue.min_factor)}>fatigue ×{n(fatigue.min_factor, 2)}</b>
              </>
            )}
          </div>
          {r.parts && r.parts.length > 0 && (
            <div className="part-results">
              {r.parts.map((p) => (
                <div key={p.name} className="part-result">
                  <span>{p.name}</span>
                  {p.floating ? (
                    <small className="bad">slides away: not held</small>
                  ) : (
                    <small>
                      {n(p.max_stress, 1)} MPa · {n(p.max_displacement, p.max_displacement < 0.1 ? 4 : 3)} mm
                    </small>
                  )}
                  <b className={tone(p.safety)}>×{n(p.safety, 2)}</b>
                </div>
              ))}
            </div>
          )}
          <dl>
            <dt>Largest stress</dt>
            <dd>{n(r.max_stress, 1)} MPa</dd>
            <dt>Moves at most</dt>
            <dd>{n(r.max_displacement, r.max_displacement < 0.1 ? 4 : 3)} mm</dd>
            {r.peak && (
              <>
                <dt>{r.peak.singular ? "Sharp inside corner" : "Peak at the surface"}</dt>
                <dd className={r.peak.singular ? "warn" : ""} title={`Re-solved on ${n(r.peak.cell, 3)} mm cubes; the coarse cubes read ${n(r.peak.coarse, 1)} MPa there`}>
                  {r.peak.singular ? (
                    "no finite peak: add a fillet"
                  ) : (
                    <>
                      {n(r.peak.stress, 1)} MPa <small className={r.peak.change > 0.1 ? "warn" : ""}>{r.peak.change > 0.1 ? "still settling" : `settled to ${n(Math.max(r.peak.change * 100, 1), 0)}%`}</small>
                    </>
                  )}
                </dd>
              </>
            )}
            {r.plastic && (
              <>
                <dt>Yielded</dt>
                <dd className={r.plastic.settled ? "" : "warn"}>
                  {r.plastic.yielded.toLocaleString("en-US")} cubes, strain up to {n(r.plastic.max_strain * 100, 2)} %
                </dd>
              </>
            )}
            {fatigue && (
              <>
                <dt>Fatigue life</dt>
                <dd title={fatigue.notes.join(" ")}>
                  {fatigue.life == null || fatigue.life > 1e15 ? "unlimited" : `${fatigue.life.toExponential(2)} cycles`} <small>Se {n(fatigue.strength, 0)} MPa</small>
                </dd>
              </>
            )}
            {r.bolts?.map((b, i) => (
              <Fragment key={`b${i}`}>
                <dt>Bolt in {b.part}</dt>
                <dd>
                  {n(b.force, 0)} N <small>preload {n(b.preload, 0)} N</small>
                </dd>
              </Fragment>
            ))}
            {r.heat && (
              <>
                <dt>Temperatures</dt>
                <dd>
                  {n(r.heat.min, 0)}–{n(r.heat.max, 0)} °C (stress-free at {n(r.heat.reference, 0)} °C)
                </dd>
                {r.heat.thermal_stress !== null && r.heat.mechanical_stress !== null && (
                  <>
                    <dt>From heat · from loads</dt>
                    <dd>
                      {n(r.heat.thermal_stress, 1)} · {n(r.heat.mechanical_stress, 1)} MPa
                    </dd>
                  </>
                )}
              </>
            )}
          </dl>
          {!!r.reactions?.length && (
            <div className="reactions" title="What each hold pushes back on the part with (N), and its moment about the hold's middle (N·m)">
              {r.reactions.map((x) => (
                <div key={x.name}>
                  <span>{x.name}</span>
                  <small>
                    {x.force.map((v) => n(v, 1)).join(", ")} N · {x.moment.map((v) => n(v / 1000, 2)).join(", ")} N·m
                  </small>
                </div>
              ))}
            </div>
          )}
          {r.contact && (
            <div className="contact-results">
              {r.contact.pairs.map((c) => (
                <div key={c.parts.join("|")} className="contact-result">
                  <span>
                    {c.parts[0]} ↔ {c.parts[1]} <small>{c.kind === "bonded" ? "glued" : c.kind === "friction" ? `μ ${c.friction}` : "sliding"}</small>
                  </span>
                  {c.press_fit && (
                    <small>
                      Press fit: {n((c.interference ?? 0) * 1000, 1)} µm interference squeezes them together at {n(c.mean_pressure, 1)} MPa on average
                    </small>
                  )}
                  <small>
                    {n(c.normal_force, 1)} N push · {n(c.shear_force, 1)} N sideways · {n(c.pressing, 0)} of {n(c.area, 0)} mm² pressing{c.slipping > 0 ? ` (${n(c.slipping, 0)} slipping)` : ""} · up to {n(c.max_pressure, 1)} MPa
                  </small>
                </div>
              ))}
            </div>
          )}
          <div className="segmented layer-switch" role="group" aria-label="Show">
            {(
              [
                ["stress", "Stress", true],
                ["safety", "Safety", true],
                ["movement", "Movement", true],
                ["contact", "Contact", !!r.contact],
                ["fatigue", "Fatigue", !!fatigue],
                ["plastic", "Yielding", !!r.plastic_strain?.length],
              ] as [Layer, string, boolean][]
            )
              .filter(([, , on]) => on)
              .map(([value, label]) => (
                <button key={value} type="button" className={layer === value ? "active" : ""} aria-pressed={layer === value} onClick={() => {
                  setLayer(value);
                  setShowing("stress");
                }}>
                  {label}
                </button>
              ))}
          </div>
          <div className={`stress-legend ${layer === "safety" || layer === "fatigue" ? "safety-legend" : ""}`} aria-hidden="true">
            {legend(r, layer, fatigue)}
          </div>
          {r.contact && layer === "contact" && r.contact.opened.length > 0 && (
            <p className="motion-note">
              <span className="gap-dot" /> Where touching faces came apart (up to {n(Math.max(...r.contact.opened.map((o) => o[3])) * 1000, 1)} µm).
            </p>
          )}
          <p className="motion-note marker-key">
            <span className="marker-dot red" /> largest stress <span className="marker-dot blue" /> largest movement
          </p>
          <label className="xray-toggle">
            <input type="checkbox" checked={deform} onChange={(e) => setDeform(e.target.checked)} /> Show the bending (exaggerated)
          </label>
          <div className="section-cut">
            <label className="xray-toggle" title="Cut the result open to see inside">
              <input type="checkbox" checked={!!section} onChange={(e) => setSection(e.target.checked && bounds ? { axis: 0, at: (bounds.min[0] + bounds.max[0]) / 2 } : null)} /> Cut open
            </label>
            {section && bounds && (
              <>
                <div className="segmented" role="group" aria-label="Cut across">
                  {(["X", "Y", "Z"] as const).map((a, k) => (
                    <button key={a} type="button" className={section.axis === k ? "active" : ""} aria-pressed={section.axis === k} onClick={() => setSection({ axis: k as 0 | 1 | 2, at: (bounds.min[k] + bounds.max[k]) / 2 })}>
                      {a}
                    </button>
                  ))}
                </div>
                <input type="range" min={bounds.min[section.axis]} max={bounds.max[section.axis]} step={(bounds.max[section.axis] - bounds.min[section.axis]) / 200 || 0.1} value={section.at} aria-label="Where the cut is" onChange={(e) => setSection({ ...section, at: Number(e.target.value) })} />
                <small>{n(section.at, 1)} mm</small>
              </>
            )}
          </div>
          {state.key && (
            <div className="probe">
              <span>Read at</span>
              {(probeAt ?? r.max_stress_at ?? [0, 0, 0]).map((v, k) => (
                <input
                  key={`p${k}:${v}`}
                  defaultValue={String(Number(v.toFixed(2)))}
                  inputMode="decimal"
                  aria-label={`${"xyz"[k]} in millimetres`}
                  onBlur={(e) => {
                    const x = Number(e.target.value);
                    if (!Number.isFinite(x)) return;
                    const next = [...(probeAt ?? r.max_stress_at ?? [0, 0, 0])] as V3;
                    next[k] = x;
                    setProbeAt(next);
                  }}
                  onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                />
              ))}
              <button type="button" className="secondary" onClick={runProbe}>
                Probe
              </button>
            </div>
          )}
          {probe?.kind === "error" && <p className="print-note bad">{probe.text}</p>}
          {probe?.kind === "done" && (
            <p className="motion-note probe-out">
              {n(probe.p.von_mises, 2)} MPa von Mises (principal {probe.p.principal.map((v) => n(v, 1)).join(", ")}) · moves {probe.p.displacement.map((v) => n(v * 1000, 2)).join(", ")} µm · safety ×{n(probe.p.safety, 2)}
              {probe.p.plastic_strain != null ? ` · plastic strain ${n(probe.p.plastic_strain * 100, 3)} %` : ""}
            </p>
          )}
          {state.key && (
            <div className="export-row">
              <a className="secondary button-link" href={projectUrl(`/results/${state.key}/csv`)} download>
                <Icon name="download" /> Table (CSV)
              </a>
              <button type="button" className="secondary" onClick={exportDeck} title="The same cubes, holds and forces as a CalculiX input deck, to cross-check in another solver">
                <Icon name="download" /> CalculiX deck
              </button>
            </div>
          )}
          <p className="motion-note">{[...r.notes, ...(fatigue?.notes ?? [])].join(" ")}</p>
        </div>
      )}
      <div className="heat-block">
        <h4>Heat</h4>
        <div className="stress-list">
          {spec.holds.map((h, i) => (
            <div key={`h${i}`} className={`stress-item hold ${resolved.holds[i]?.face === null ? "lost" : ""}`}>
              <span className="stress-dot" />
              Held at
              <input
                defaultValue={String(h.temperature)}
                key={`${i}:${h.temperature}`}
                inputMode="decimal"
                aria-label="Temperature in degrees Celsius"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== h.temperature) save({ ...spec, holds: spec.holds.map((x, j) => (j === i ? { ...x, temperature: v } : x)) });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              °C
              <button type="button" aria-label="Remove" onClick={() => save({ ...spec, holds: spec.holds.filter((_, j) => j !== i) })}>
                <Icon name="close" />
              </button>
            </div>
          ))}
          {spec.heats.map((h, i) => (
            <div key={`q${i}`} className={`stress-item heat ${resolved.heats[i]?.face === null ? "lost" : ""}`}>
              <span className="stress-dot" />
              <input
                defaultValue={String(h.watts)}
                key={`${i}:${h.watts}`}
                inputMode="decimal"
                aria-label="Heat in watts"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== h.watts) save({ ...spec, heats: spec.heats.map((x, j) => (j === i ? { ...x, watts: v } : x)) });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              W coming in
              <button type="button" aria-label="Remove" onClick={() => save({ ...spec, heats: spec.heats.filter((_, j) => j !== i) })}>
                <Icon name="close" />
              </button>
            </div>
          ))}
        </div>
        <div className="stress-add">
          <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face held at a temperature (a heatsink, a wall)", (f) => save({ ...spec, holds: [...spec.holds, { face: pickOf(f), temperature: 20 }] }))}>
            <Icon name="plus" /> Held at °C
          </button>
          <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the heat comes in through (a motor, a chip)", (f) => save({ ...spec, heats: [...spec.heats, { face: pickOf(f), watts: 5 }] }))}>
            <Icon name="plus" /> Heat in
          </button>
          <div className="segmented" role="group" aria-label="Air cooling">
            {AIR.map((a) => (
              <button
                key={a.h}
                type="button"
                className={(spec.convection?.h ?? 0) === a.h ? "active" : ""}
                aria-pressed={(spec.convection?.h ?? 0) === a.h}
                title={a.h ? `${a.h} W/m²·K, air at ${spec.convection?.ambient ?? 20} °C` : "No cooling by air"}
                onClick={() => save({ ...spec, convection: a.h ? { h: a.h, ambient: spec.convection?.ambient ?? 20 } : null })}
              >
                {a.label}
              </button>
            ))}
          </div>
        </div>
        <label className="xray-toggle" title="Thermal stress: the part expands where it is warmer than the stress-free temperature, and is stressed where it cannot">
          <input type="checkbox" checked={!!spec.heat_stress} onChange={(e) => save({ ...spec, heat_stress: e.target.checked ? { reference: spec.heat_stress?.reference ?? 20 } : null })} /> Include this heat in the stress study
          {spec.heat_stress && (
            <>
              , stress-free at
              <input
                className="heat-reference"
                defaultValue={String(spec.heat_stress.reference)}
                key={`ref:${spec.heat_stress.reference}`}
                inputMode="decimal"
                aria-label="Stress-free temperature in degrees Celsius"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== spec.heat_stress?.reference) save({ ...spec, heat_stress: { reference: v } });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              °C
            </>
          )}
        </label>
        <button type="button" className="secondary stress-run heat-run" disabled={(!holds.length && !(spec.convection && spec.convection.h > 0)) || heat?.kind === "busy" || !props.material} onClick={runHeat} title="Steady temperatures: where it gets hot, and where the heat goes">
          <Icon name="alert" /> {heat?.kind === "busy" ? "Solving…" : "Heat"}
        </button>
        {heat?.kind === "error" && <p className="print-note bad">{heat.text}</p>}
        {heat?.kind === "done" && (
          <div className="stress-result">
            <dl>
              <dt>Hottest</dt>
              <dd>{n(heat.temps.max, 1)} °C</dd>
              <dt>Coolest</dt>
              <dd>{n(heat.temps.min, 1)} °C</dd>
              <dt>Heat in</dt>
              <dd>{n(heat.temps.heat_in, 2)} W</dd>
              <dt>Out to holds · air</dt>
              <dd>
                {n(heat.temps.to_holds, 2)} W · {n(heat.temps.to_air, 2)} W
              </dd>
            </dl>
            <div className="stress-legend heat-legend" aria-hidden="true">
              <span>{n(heat.temps.min, 0)} °C</span>
              <i />
              <span>{n(heat.temps.max, 0)} °C</span>
            </div>
            {showing !== "heat" && (
              <button type="button" className="link-button" onClick={() => setShowing("heat")}>
                Show the temperatures
              </button>
            )}
            <p className="motion-note">{heat.temps.notes.join(" ")}</p>
          </div>
        )}
      </div>
    </section>
  );
}

/** The result as the chosen layer colours it: each cube's figure and the top of the scale. */
function layered(r: Result, layer: Layer, fatigue: Fatigue | null | undefined): Result {
  if (layer === "safety") {
    // How near yield each cube is: von Mises over its part's yield (red at yield).
    const yields = r.parts?.map((p) => p.yield_strength);
    return { ...r, cells: r.cells.map(([c, v], i) => [c, v / (yields?.[r.cell_parts?.[i] ?? 0] ?? r.yield_strength)] as [V3, number]), max_stress: 1 };
  }
  if (layer === "movement") {
    // Each cube's movement: the mean of its corners'.
    const several = !!r.cell_parts?.length && !!r.node_parts?.length;
    const moved = new Map(r.displacements.map(([c, d], i) => [`${several ? r.node_parts![i] : 0}:${c[0]},${c[1]},${c[2]}`, Math.hypot(...d)]));
    const cells = r.cells.map(([c], i) => {
      const part = several ? r.cell_parts![i] : 0;
      let sum = 0;
      for (let k = 0; k < 8; k++) sum += moved.get(`${part}:${c[0] + (k & 1)},${c[1] + ((k >> 1) & 1)},${c[2] + ((k >> 2) & 1)}`) ?? 0;
      return [c, sum / 8] as [V3, number];
    });
    return { ...r, cells, max_stress: Math.max(r.max_displacement, 1e-12) };
  }
  if (layer === "fatigue" && fatigue) {
    return { ...r, cells: r.cells.map(([c], i) => [c, 1 / Math.max(fatigue.factors[i] ?? 1e9, 1e-9)] as [V3, number]), max_stress: 1 };
  }
  if (layer === "plastic" && r.plastic_strain?.length) {
    const top = Math.max(...r.plastic_strain, 1e-12);
    return { ...r, cells: r.cells.map(([c], i) => [c, r.plastic_strain![i] ?? 0] as [V3, number]), max_stress: top };
  }
  return r;
}

/** The legend under the map for a layer. */
function legend(r: Result, layer: Layer, fatigue: Fatigue | null | undefined) {
  const ends: [string, string] =
    layer === "safety"
      ? ["×∞", "×1 yields"]
      : layer === "movement"
        ? ["0", `${n(r.max_displacement, r.max_displacement < 0.1 ? 4 : 3)} mm`]
        : layer === "fatigue" && fatigue
          ? ["×∞", "×1 fails at the life"]
          : layer === "plastic" && r.plastic
            ? ["0", `${n(r.plastic.max_strain * 100, 2)} % plastic strain`]
            : layer === "contact" && r.contact
              ? ["0", `${n(r.contact.max_pressure, 1)} MPa`]
              : ["0", `${n(r.max_stress, 0)} MPa`];
  return (
    <>
      <span>{ends[0]}</span>
      <i />
      <span>{ends[1]}</span>
    </>
  );
}
