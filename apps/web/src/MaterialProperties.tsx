// A part's material as the studies see it: the library's typical figures
// for its name, with where they come from, and any the person set on the
// material laid over them. Every figure can be edited; the edit is kept on
// the project's material (every part made of it, and every study and check
// using it, follows), and "Back to typical" clears them.

import { useEffect, useState } from "react";
import { projectUrl } from "./engine";

/** As the engine's material library gives it (MPa, kg/m³, W/m·K, J/kg·K, 1/K). */
export type MaterialProps = {
  name: string;
  family: string;
  e: number;
  nu: number;
  density: number;
  yield_strength: number;
  uts: number;
  elongation: number;
  tangent_modulus: number;
  fatigue_strength: number | null;
  fatigue_cycles: number;
  endurance_limit: boolean;
  conductivity: number;
  specific_heat: number;
  expansion: number;
  max_temperature: number | null;
  source: string;
  edited?: string[];
};

type Own = { id: string; name: string; set: Record<string, number | string> | null; properties: MaterialProps | null };

/** Each figure: its key on the material, label, unit, and the scale shown (E in GPa, α in µm/m·K). */
const FIELDS: { key: keyof MaterialProps & string; label: string; unit: string; scale: number; digits: number; title: string }[] = [
  { key: "e", label: "E", unit: "GPa", scale: 1e-3, digits: 2, title: "Young's modulus: how stiff it is" },
  { key: "nu", label: "ν", unit: "", scale: 1, digits: 3, title: "Poisson's ratio: how much it narrows when stretched" },
  { key: "yield_strength", label: "Yield", unit: "MPa", scale: 1, digits: 1, title: "Where it starts to bend for good: safety factors are against this" },
  { key: "uts", label: "Tensile", unit: "MPa", scale: 1, digits: 1, title: "Ultimate tensile strength: where it breaks" },
  { key: "tangent_modulus", label: "Past yield", unit: "MPa", scale: 1, digits: 0, title: "Tangent modulus: the slope of stress against strain once it yields (bilinear plasticity)" },
  { key: "fatigue_strength", label: "Fatigue", unit: "MPa", scale: 1, digits: 1, title: "Fully reversed fatigue strength of a polished sample at the cycles beside it" },
  { key: "fatigue_cycles", label: "at cycles", unit: "", scale: 1, digits: 0, title: "The life the fatigue strength is for" },
  { key: "conductivity", label: "k", unit: "W/m·K", scale: 1, digits: 3, title: "Thermal conductivity" },
  { key: "specific_heat", label: "c", unit: "J/kg·K", scale: 1, digits: 0, title: "Specific heat: what heat over time needs" },
  { key: "expansion", label: "α", unit: "µm/m·K", scale: 1e6, digits: 1, title: "Thermal expansion" },
];

const show = (v: number | null | undefined, digits: number) => (v == null ? "" : String(Number(v.toFixed(digits))));

export function MaterialProperties(props: { material: string | null; version: unknown; run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>; onChange?: () => void }) {
  const [own, setOwn] = useState<Own | null>(null);
  const [typical, setTypical] = useState<MaterialProps | null>(null);
  const [open, setOpen] = useState(false);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!props.material) return;
    let live = true;
    fetch(projectUrl("/materials"))
      .then((r) => (r.ok ? r.json() : null))
      .then((m: { library: MaterialProps[]; project: Own[] } | null) => {
        if (!live || !m) return;
        const name = props.material!.toLowerCase();
        setOwn(m.project.find((x) => x.name.toLowerCase() === name) ?? null);
        // The library entry its name falls under (by its first word, in the library's order, as the engine matches).
        setTypical(m.library.find((x) => name.includes(x.name.split(/[ ,(]/)[0].toLowerCase())) ?? null);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [props.material, props.version, tick]);
  if (!props.material) return null;
  const p = own?.properties ?? typical;
  const set = (key: string, value: number | null) => {
    if (!own) return;
    const next: Record<string, number | string> = { ...(own.set ?? {}) };
    if (value === null) delete next[key];
    else next[key] = value;
    void props.run("solid.material.set_properties", { target: own.id, properties: Object.keys(next).length ? next : null }).then(() => {
      setTick((t) => t + 1);
      props.onChange?.();
    });
  };
  return (
    <div className="material-block">
      <button type="button" className="material-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>Material</span>
        <b>{p ? p.name : props.material}</b>
        <small>{p ? `${show(p.e / 1000, 1)} GPa · yield ${show(p.yield_strength, 0)} MPa${p.edited?.length ? " · edited" : " · typical"}` : "no figures: set them"}</small>
      </button>
      {open && (
        <div className="material-body">
          {!own && <p className="motion-note">“{props.material}” is not one of the project's materials, so its figures are the library's (read only here).</p>}
          <div className="material-grid">
            {FIELDS.map((f) => {
              const value = p ? (p[f.key] as number | null) : null;
              const edited = own?.set && f.key in own.set;
              return (
                <label key={f.key} className={edited ? "edited" : ""} title={f.title}>
                  <span>{f.label}</span>
                  <input
                    key={`${f.key}:${value}:${tick}`}
                    defaultValue={value == null ? "" : show(value * f.scale, f.digits)}
                    inputMode="decimal"
                    disabled={!own}
                    aria-label={`${f.title}${f.unit ? ` (${f.unit})` : ""}`}
                    onBlur={(e) => {
                      const raw = e.target.value.trim();
                      if (raw === "") return edited && set(f.key, null);
                      const v = Number(raw) / f.scale;
                      if (Number.isFinite(v) && (value == null || Math.abs(v - value) > 1e-12 * Math.max(1, Math.abs(value)))) set(f.key, v);
                    }}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                  <small>{f.unit}</small>
                </label>
              );
            })}
            <label title="Density sets its mass (and gravity, spinning, vibration); change it on the material">
              <span>ρ</span>
              <input value={p ? show(p.density, 0) : ""} disabled readOnly aria-label="Density (kg/m³)" />
              <small>kg/m³</small>
            </label>
          </div>
          {p && <p className="material-source">{p.source}</p>}
          {own?.set && (
            <button type="button" className="link-button" onClick={() => void props.run("solid.material.set_properties", { target: own.id, properties: null }).then(() => setTick((t) => t + 1))}>
              Back to typical figures
            </button>
          )}
        </div>
      )}
    </div>
  );
}
