// Aluminium flat bar, EN AW-6060 T6, cut to length.
//
// Sources: EN 755-9:2016 (extruded aluminium profiles: tolerances on form
// and dimensions) with the sizes merchants stock in EN AW-6060 T6 / 6063 T6
// (EN 755-2 properties). Mass per metre from the modelled section at
// 2700 kg/m³. Mill finish, sold by the metre and sawn to length.
// Edges drawn square.
//
// Stands on its end: centred on the origin, the width along X, the
// thickness along Y, the length along +Z from z = 0.

const b = p.b;
const t = p.t;
const L = p.length;
const outline = roundedOutline([[-b / 2, -t / 2], [b / 2, -t / 2], [b / 2, t / 2], [-b / 2, t / 2]]);
const { solid, props } = member(outline, L);
memberConnectors(L);
faceConnector("Face +Y", 0, t / 2, L, [0, 1]);
faceConnector("Face -Y", 0, -t / 2, L, [0, -1]);
faceConnector("Edge +X", b / 2, 0, L, [1, 0]);
faceConnector("Edge -X", -b / 2, 0, L, [-1, 0]);
const designation = `Al flat ${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "EN AW-6060 T6",
  standard: "EN 755-9",
  description: `Aluminium flat bar ${num(b)}×${num(t)}, extruded`,
  kgPerM: props.A * ALU_KG_PER_M_PER_MM2,
  rate: "aluminium",
  L,
  sources: aluminiumSources(`aluminium flat bar ${num(b)}x${num(t)}`),
});
return solid;
