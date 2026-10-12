// Aluminium square and rectangular tube, EN AW-6060 T6, cut to length.
//
// Sources: EN 755-9:2016 (extruded aluminium profiles: tolerances on form
// and dimensions) with the sizes merchants stock in EN AW-6060 T6 / 6063 T6
// (EN 755-2 properties). Mass per metre from the modelled section at
// 2700 kg/m³. Mill finish, sold by the metre and sawn to length.
// Extruded tubes have near-square corners: drawn with a 0.5 mm outside
// radius and a sharp inside corner.
//
// Stands on its end: centred on the origin, the depth h along Y, the width
// b along X, the length along +Z from z = 0.

const h = p.h;
const b = p.b;
const t = p.t;
const L = p.length;
const { solid, props } = hollowSection({ b, h, t, ro: 0.5, ri: 0 }, L);
const shape = h === b ? "square" : "rectangular";
const designation = `Al ${shape} tube ${num(h)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "EN AW-6060 T6",
  standard: "EN 755-9",
  description: `Aluminium ${shape} tube ${num(h)}×${num(b)}×${num(t)}, extruded`,
  kgPerM: props.A * ALU_KG_PER_M_PER_MM2,
  rate: "aluminium",
  L,
  sources: aluminiumSources(`aluminium ${shape} tube ${num(h)}x${num(b)}x${num(t)}`),
});
metric("Wall t", t, "mm");
return solid;
