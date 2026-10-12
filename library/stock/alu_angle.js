// Aluminium angle (equal and unequal legs), EN AW-6060 T6, cut to length.
//
// Sources: EN 755-9:2016 (extruded aluminium profiles: tolerances on form
// and dimensions) with the sizes merchants stock in EN AW-6060 T6 / 6063 T6
// (EN 755-2 properties). Mass per metre from the modelled section at
// 2700 kg/m³. Mill finish, sold by the metre and sawn to length.
// Extruded angles have square heels and toes and a small root radius;
// drawn here as r = t/2.
//
// Stands on its end: the heel on the origin, leg a up +Y, leg b along +X,
// the length along +Z from z = 0. The axis connector is at the centroid.

const a = p.a;
const b = p.b;
const t = p.t;
const L = p.length;
const { solid, props } = angleSection({ a, b, t, r1: t / 2 }, L);
const designation = `Al angle ${num(a)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "EN AW-6060 T6",
  standard: "EN 755-9",
  description: `Aluminium angle ${num(a)}×${num(b)}×${num(t)}, extruded`,
  kgPerM: props.A * ALU_KG_PER_M_PER_MM2,
  rate: "aluminium",
  L,
  sources: aluminiumSources(`aluminium angle ${num(a)}x${num(b)}x${num(t)}`),
});
return solid;
