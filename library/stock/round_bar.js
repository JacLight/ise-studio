// Hot-rolled round bar, EN 10060, cut to length.
//
// Sources: EN 10060:2003, hot-rolled round steel bars for general purposes:
// diameter d (the standard's preferred sizes, as merchant-bar catalogues
// stock them); mass per metre G = π·d²/4 at 7850 kg/m³, as the standard
// computes it. S235JR to EN 10025-2, sold by the metre and sawn to length.
//
// Stands on its end: the axis on Z, the length along +Z from z = 0.

const d = p.d;
const L = p.length;
const solid = circle(d / 2).extrude(L);
const A = (Math.PI * d * d) / 4;
const I = (Math.PI * d ** 4) / 64;
metric("Section area", Math.round(A) / 100, "cm²");
metric("Iy", Math.round(I / 100) / 100, "cm⁴");
metric("Wel,y", Math.round(I / (d / 2) / 10) / 100, "cm³");
memberConnectors(L);
faceConnector("Surface +X", d / 2, 0, L, [1, 0]);
faceConnector("Surface +Y", 0, d / 2, L, [0, 1]);
const designation = `RD ${num(d)}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10060",
  description: `Round bar Ø${num(d)}, hot rolled EN 10060`,
  kgPerM: A * STEEL_KG_PER_M_PER_MM2,
  rate: "bar",
  L,
  sources: steelSources(`round bar ${num(d)}mm steel`),
});
metric("Diameter d", d, "mm");
return solid;
