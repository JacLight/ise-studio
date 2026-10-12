// Cold-formed structural hollow section, SHS, EN 10219-2, cut to length.
//
// Sources: EN 10219-2 (tolerances, dimensions and sectional properties) and
// EN 10219-1 (delivery conditions, S355J2H):
// outside dimensions H × B, wall T, corner radii ro / ri;
// the sizes are the common stocked ones. Mass per metre from the
// standard's formula at 7850 kg/m³:
//   A = 2T(B + H − 2T) − (4 − π)(ro² − ri²)
// The modelled section is that outline with true arcs, so it matches.
//
// Stands on its end: the depth h along Y, the width b along X, centred on the origin,
// the length along +Z from z = 0.

// EN 10219-2 corner radii for calculation (Table A.1 note / Annex B):
// T ≤ 6: ro = 2.0 T, ri = 1.0 T; 6 < T ≤ 10: ro = 2.5 T, ri = 1.5 T;
// T > 10: ro = 3.0 T, ri = 2.0 T.
function cornerRadii(t) {
  if (t <= 6) return [2 * t, t];
  if (t <= 10) return [2.5 * t, 1.5 * t];
  return [3 * t, 2 * t];
}

const b = p.b;
const h = p.b;
const t = p.t;
const L = p.length;
const [ro, ri] = cornerRadii(t);
const { solid } = hollowSection({ b, h, t, ro, ri }, L);
const A = 2 * t * (b + h - 2 * t) - (4 - Math.PI) * (ro * ro - ri * ri);
const designation = `CF SHS ${num(b)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S355J2H",
  standard: "EN 10219-2",
  description: `${designation}, cold-formed EN 10219-2`,
  kgPerM: A * STEEL_KG_PER_M_PER_MM2,
  rate: "coldHollow",
  L,
  sources: steelSources(`SHS ${num(b)}x${num(b)}x${num(t)} box section`),
});
metric("Wall T", t, "mm");
metric("Outer corner radius ro", ro, "mm");
metric("Inner corner radius ri", ri, "mm");
return solid;
