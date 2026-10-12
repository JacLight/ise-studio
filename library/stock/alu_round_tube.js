// Aluminium round tube, EN AW-6060 T6, cut to length.
//
// Sources: EN 755-9:2016 (extruded aluminium profiles: tolerances on form
// and dimensions) with the sizes merchants stock in EN AW-6060 T6 / 6063 T6
// (EN 755-2 properties). Mass per metre from the modelled section at
// 2700 kg/m³. Mill finish, sold by the metre and sawn to length.
//
// Stands on its end: the axis on Z, the length along +Z from z = 0.

const d = p.d;
const t = p.t;
const L = p.length;
const { solid } = hollowSection({ d, t }, L);
const designation = `Al tube Ø${num(d)}×${num(t)}`;
cutToLength({
  designation,
  grade: "EN AW-6060 T6",
  standard: "EN 755-9",
  description: `Aluminium round tube Ø${num(d)}×${num(t)}, extruded`,
  kgPerM: Math.PI * t * (d - t) * ALU_KG_PER_M_PER_MM2,
  rate: "aluminium",
  L,
  sources: aluminiumSources(`aluminium round tube ${num(d)}x${num(t)}`),
});
metric("Outside diameter", d, "mm");
metric("Wall t", t, "mm");
return solid;
