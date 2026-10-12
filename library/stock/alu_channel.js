// Aluminium channel (U profile, equal flanges), EN AW-6060 T6, cut to length.
//
// Sources: EN 755-9:2016 (extruded aluminium profiles: tolerances on form
// and dimensions) with the sizes merchants stock in EN AW-6060 T6 / 6063 T6
// (EN 755-2 properties). Mass per metre from the modelled section at
// 2700 kg/m³. Mill finish, sold by the metre and sawn to length.
// Web and flanges one thickness t, square toes, root radius drawn as t/2.
//
// Stands on its end: the back of the web on x = 0, the flanges toward +X,
// the depth h along Y centred on y = 0, the length along +Z from z = 0.

const h = p.h;
const b = p.b;
const t = p.t;
const L = p.length;
const { solid, props } = channelSection({ h, b, tw: t, tf: t, r1: t / 2 }, L);
const designation = `Al channel ${num(h)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "EN AW-6060 T6",
  standard: "EN 755-9",
  description: `Aluminium channel ${num(h)}×${num(b)}×${num(t)} (web × flange × thickness), extruded`,
  kgPerM: props.A * ALU_KG_PER_M_PER_MM2,
  rate: "aluminium",
  L,
  sources: aluminiumSources(`aluminium channel ${num(h)}x${num(b)}x${num(t)}`),
});
return solid;
