// Hot-rolled flat bar, EN 10058, cut to length.
//
// Sources: EN 10058:2018, hot-rolled flat steel bars for general purposes:
// width b × thickness t (the standard's preferred sizes, as merchant-bar
// catalogues stock them); mass per metre G = b·t at 7850 kg/m³, as the
// standard computes it. Edges are drawn square (rolled edges are slightly
// rounded). S235JR to EN 10025-2, sold by the metre and sawn to length.
//
// Stands on its end: the section centred on the origin, the width along X,
// the thickness along Y, the length along +Z from z = 0.

const b = p.b;
const t = p.t;
const L = p.length;
const outline = roundedOutline([[-b / 2, -t / 2], [b / 2, -t / 2], [b / 2, t / 2], [-b / 2, t / 2]]);
const { solid } = member(outline, L);
memberConnectors(L);
faceConnector("Face +Y", 0, t / 2, L, [0, 1]);
faceConnector("Face -Y", 0, -t / 2, L, [0, -1]);
faceConnector("Edge +X", b / 2, 0, L, [1, 0]);
faceConnector("Edge -X", -b / 2, 0, L, [-1, 0]);
const designation = `FL ${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10058",
  description: `Flat bar ${num(b)}×${num(t)}, hot rolled EN 10058`,
  kgPerM: b * t * STEEL_KG_PER_M_PER_MM2,
  rate: "bar",
  L,
  sources: steelSources(`flat bar ${num(b)}x${num(t)} steel`),
});
return solid;
