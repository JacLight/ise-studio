// Hot-finished structural hollow section, SHS, EN 10210-2, cut to length.
//
// Sources: EN 10210-2 (tolerances, dimensions and sectional properties) and
// EN 10210-1 (delivery conditions, S355J2H):
// outside dimensions H × B, wall T, corner radii ro / ri;
// the sizes are the common stocked ones. Mass per metre from the
// standard's formula at 7850 kg/m³:
//   A = 2T(B + H − 2T) − (4 − π)(ro² − ri²)
// The modelled section is that outline with true arcs, so it matches.
//
// Stands on its end: the depth h along Y, the width b along X, centred on the origin,
// the length along +Z from z = 0.

// EN 10210-2 corner radii for calculation: ro = 1.5 T, ri = 1.0 T.
function cornerRadii(t) {
  return [1.5 * t, t];
}

const b = p.b;
const h = p.b;
const t = p.t;
const L = p.length;
const [ro, ri] = cornerRadii(t);
const { solid } = hollowSection({ b, h, t, ro, ri }, L);
const A = 2 * t * (b + h - 2 * t) - (4 - Math.PI) * (ro * ro - ri * ri);
const designation = `HF SHS ${num(b)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S355J2H",
  standard: "EN 10210-2",
  description: `${designation}, hot-finished EN 10210-2`,
  kgPerM: A * STEEL_KG_PER_M_PER_MM2,
  rate: "hotHollow",
  L,
  sources: steelSources(`SHS ${num(b)}x${num(b)}x${num(t)} box section`),
});
metric("Wall T", t, "mm");
metric("Outer corner radius ro", ro, "mm");
metric("Inner corner radius ri", ri, "mm");
return solid;
