// Hot-finished structural hollow section, CHS, EN 10210-2, cut to length.
//
// Sources: EN 10210-2 (tolerances, dimensions and sectional properties) and
// EN 10210-1 (delivery conditions, S355J2H):
// outside diameter D, wall T;
// the sizes are the common stocked ones. Mass per metre from the
// standard's formula at 7850 kg/m³:
//   A = π·T·(D − T)
// The modelled section is that outline with true arcs, so it matches.
//
// Stands on its end: the axis on Z,
// the length along +Z from z = 0.

const d = p.d;
const t = p.t;
const L = p.length;
const { solid } = hollowSection({ d, t }, L);
const A = Math.PI * t * (d - t);
const designation = `HF CHS ${num(d)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S355J2H",
  standard: "EN 10210-2",
  description: `${designation}, hot-finished EN 10210-2`,
  kgPerM: A * STEEL_KG_PER_M_PER_MM2,
  rate: "hotHollow",
  L,
  sources: steelSources(`CHS ${num(d)}x${num(t)} tube steel`),
});
metric("Outside diameter D", d, "mm");
metric("Wall T", t, "mm");
return solid;
